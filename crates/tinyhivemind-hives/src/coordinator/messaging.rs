//! Atomic acceptance, retry identity, and visibility admission.
use super::{
    Coordinator, Destination, HOST_ID, Message, Receipt, SendMessage, identifier, known_agent,
};
use crate::{Delivery, DeliveryStatus, EpisodeRecord, Error, Result, StoredState};
use std::collections::BTreeSet;
impl Coordinator {
    /// Accept and enqueue a message without waiting for a response.
    /// Sends by an active scheduled agent inherit its captured scheduled authority.
    /// # Errors
    /// Returns unknown sender/destination, missing membership, invalid thread,
    /// conflicting retry identity or persistence errors.
    pub async fn send(&self, request: SendMessage) -> Result<Receipt> {
        if request.sender == HOST_ID {
            return Err(Error::InvalidIdentifier("sender"));
        }
        self.accept(request, None).await
    }
    /// Submit through the reserved host identity rather than impersonating an agent.
    /// The request's sender field is overwritten.
    /// # Errors
    /// Returns destination, visibility, retry or storage validation errors.
    pub async fn send_as_host(&self, mut request: SendMessage) -> Result<Receipt> {
        request.sender = HOST_ID.into();
        self.accept(request, None).await
    }
    /// Accept host work carrying durable scheduled authority.
    /// # Errors
    /// Returns invalid job identity, destination, retry, visibility or storage errors.
    pub async fn send_scheduled_as_host(
        &self,
        job_id: &str,
        mut request: SendMessage,
    ) -> Result<Receipt> {
        identifier(job_id, "scheduled job id")?;
        request.sender = HOST_ID.into();
        self.accept(request, Some(job_id.to_owned())).await
    }
    async fn accept(
        &self,
        request: SendMessage,
        scheduled_job_id: Option<String>,
    ) -> Result<Receipt> {
        identifier(&request.message_id, "message id")?;
        if request.message_id.starts_with("hivemind:") {
            return Err(Error::InvalidIdentifier("reserved message id"));
        }
        let retention = self.inner.options.retention;
        self.update(|state| {
            let scheduled_job_id = scheduled_job_id.clone().or_else(|| {
                state
                    .running
                    .get(&request.sender)
                    .and_then(|running| running.request.scheduled_job_id.clone())
            });
            if let Some(receipt) = retry_receipt(state, &request, scheduled_job_id.as_deref())? {
                return Ok(receipt);
            }
            let recipients = recipients(state, &request)?;
            let only_for = narrow_readers(
                state,
                &request.destination,
                request.thread,
                request.only_for.clone(),
            )?;
            let sequence = next_sequence(state)?;
            let episode_id = matches!(request.destination, Destination::Hive(_))
                .then(|| format!("episode:{sequence}"));
            let message = Message {
                scheduled_job_id: scheduled_job_id.clone(),
                message_id: request.message_id.clone(),
                sequence,
                sender: request.sender.clone(),
                destination: request.destination.clone(),
                body: request.body.clone(),
                thread: request.thread,
                episode_id: episode_id.clone(),
                only_for,
            };
            match &request.destination {
                Destination::Agent(_) => {
                    for agent_id in &recipients {
                        retention.admit_pending(state, agent_id)?;
                    }
                    for agent_id in recipients {
                        state.deliveries.push(Delivery {
                            sequence,
                            agent_id,
                            status: DeliveryStatus::Pending,
                        });
                    }
                }
                Destination::Hive(id) => {
                    let mut hive = state
                        .hives
                        .get(id)
                        .cloned()
                        .ok_or_else(|| Error::UnknownHive(id.clone()))?;
                    if request.thread.is_some() {
                        let scope = thread_readers(state, &request.destination, request.thread)?;
                        if !scope.is_empty() {
                            hive.members.retain(|member| scope.contains(member));
                        }
                    }
                    let id = episode_id
                        .ok_or_else(|| Error::InvalidState("hive message has no episode".into()))?;
                    state.episodes.push(EpisodeRecord {
                        settings: state.hive_settings.get(&hive.hive_id).cloned(),
                        scheduled_job_id: scheduled_job_id.clone(),
                        episode_id: id,
                        hive,
                        opened_at: sequence,
                        thread: request.thread,
                        starters: if request.starters.is_empty() {
                            recipients.clone()
                        } else {
                            request.starters.clone()
                        },
                        conductor: None,
                        pending: Vec::new(),
                        wave_open: false,
                        waiting: false,
                        finished: false,
                        failure: None,
                    });
                }
            }
            state.messages.push(message);
            state
                .accepted
                .insert(request.message_id.clone(), request.clone());
            Ok(Receipt {
                message_id: request.message_id.clone(),
                sequence,
            })
        })
        .await
    }
    /// Read the caller's direct conversation with a registered peer in durable
    /// sequence order, including replies returned by the peer's runner.
    /// `after` excludes that sequence. Returned replies do not schedule another
    /// turn; callers can send an explicit follow-up when needed.
    /// # Errors
    /// Returns unknown caller/peer or a poisoned shared lock.
    pub fn read_direct(
        &self,
        agent_id: &str,
        peer_id: &str,
        after: Option<u64>,
    ) -> Result<Vec<Message>> {
        let live = self.lock()?;
        known_agent(&live.durable, agent_id)?;
        known_agent(&live.durable, peer_id)?;
        Ok(live
            .durable
            .messages
            .iter()
            .filter(|message| {
                after.is_none_or(|cursor| message.sequence > cursor)
                    && ((message.sender == agent_id
                        && message.destination == Destination::Agent(peer_id.into()))
                        || (message.sender == peer_id
                            && message.destination == Destination::Agent(agent_id.into())))
            })
            .cloned()
            .collect())
    }
    /// Read only accessible rows of a hive in durable sequence order.
    /// `after` is exclusive; a thread read includes its root and replies.
    /// # Errors
    /// Returns unknown hive/agent, unauthorized membership or invalid thread.
    pub fn read_hive(
        &self,
        agent_id: &str,
        hive_id: &str,
        after: Option<u64>,
        thread: Option<u64>,
    ) -> Result<Vec<Message>> {
        let live = self.lock()?;
        known_agent(&live.durable, agent_id)?;
        membership(&live.durable, agent_id, hive_id)?;
        let destination = Destination::Hive(hive_id.into());
        if let Some(root) = thread
            && !live.durable.messages.iter().any(|msg| {
                msg.sequence == root
                    && msg.destination == destination
                    && visible_in(&live.durable, msg, agent_id)
            })
        {
            return Err(Error::InvalidThread(root));
        }
        Ok(live
            .durable
            .messages
            .iter()
            .filter(|msg| {
                msg.destination == destination
                    && visible_in(&live.durable, msg, agent_id)
                    && after.is_none_or(|after| msg.sequence > after)
                    && thread.is_none_or(|root| msg.sequence == root || msg.thread == Some(root))
            })
            .cloned()
            .collect())
    }
}
pub(super) fn membership(state: &StoredState, agent_id: &str, hive_id: &str) -> Result<()> {
    let hive = state
        .hives
        .get(hive_id)
        .ok_or_else(|| Error::UnknownHive(hive_id.into()))?;
    let captured = state.running.get(agent_id).is_some_and(|running| {
        running
            .request
            .memberships
            .iter()
            .any(|h| h.hive_id == hive_id)
    });
    if hive.members.iter().any(|id| id == agent_id) || captured {
        Ok(())
    } else {
        Err(Error::NotMember {
            agent_id: agent_id.into(),
            hive_id: hive_id.into(),
        })
    }
}
pub(super) fn visible(message: &Message, agent_id: &str) -> bool {
    message.only_for.is_empty()
        || message.sender == agent_id
        || message.only_for.iter().any(|id| id == agent_id)
}
pub(super) fn next_sequence(state: &mut StoredState) -> Result<u64> {
    let sequence = state.next_sequence;
    state.next_sequence = sequence.checked_add(1).ok_or(Error::Exhausted)?;
    Ok(sequence)
}

fn recipients(state: &StoredState, request: &SendMessage) -> Result<Vec<String>> {
    if request.sender != HOST_ID {
        known_agent(state, &request.sender)?;
    }
    Ok(match &request.destination {
        Destination::Agent(id) => {
            known_agent(state, id)?;
            if request.thread.is_some()
                || !request.only_for.is_empty()
                || !request.starters.is_empty()
            {
                return Err(Error::InvalidIdentifier("direct message attribution"));
            }
            vec![id.clone()]
        }
        Destination::Hive(id) => {
            let hive = state
                .hives
                .get(id)
                .ok_or_else(|| Error::UnknownHive(id.clone()))?;
            if hive.members.is_empty() {
                return Err(Error::EmptyHive(id.clone()));
            }
            if request.sender != HOST_ID {
                membership(state, &request.sender, id)?;
            }
            let mut unique = BTreeSet::new();
            for recipient in &request.only_for {
                if !hive.members.contains(recipient) || !unique.insert(recipient) {
                    return Err(Error::NotMember {
                        agent_id: recipient.clone(),
                        hive_id: id.clone(),
                    });
                }
            }
            if let Some(root) = request.thread
                && !state.messages.iter().any(|msg| {
                    msg.sequence == root
                        && msg.destination == request.destination
                        && (request.sender == HOST_ID || visible_in(state, msg, &request.sender))
                })
            {
                return Err(Error::InvalidThread(root));
            }
            let scope = thread_readers(state, &request.destination, request.thread)?;
            let selected = if request.only_for.is_empty() {
                hive.members.clone()
            } else {
                request.only_for.clone()
            };
            if !scope.is_empty()
                && request
                    .only_for
                    .iter()
                    .any(|recipient| !scope.contains(recipient))
            {
                return Err(Error::InvalidThread(request.thread.unwrap_or_default()));
            }
            let selected: Vec<_> = selected
                .into_iter()
                .filter(|recipient| scope.is_empty() || scope.contains(recipient))
                .collect();
            starters_among(&request.starters, &selected, id)?;
            selected
        }
    })
}
/// Host-chosen starters must be distinct readers of the message.
fn starters_among(starters: &[String], readers: &[String], hive_id: &str) -> Result<()> {
    let mut unique = BTreeSet::new();
    for starter in starters {
        if !readers.contains(starter) {
            return Err(Error::NotMember {
                agent_id: starter.clone(),
                hive_id: hive_id.into(),
            });
        }
        if !unique.insert(starter) {
            return Err(Error::DuplicateMember(starter.clone()));
        }
    }
    Ok(())
}

/// Root-private threads can only be read by their original participants.
pub(super) fn visible_in(state: &StoredState, message: &Message, agent_id: &str) -> bool {
    if !visible(message, agent_id) {
        return false;
    }
    let mut thread = message.thread;
    for _ in 0..state.messages.len() {
        let Some(root) = thread else {
            return true;
        };
        let Some(parent) = state
            .messages
            .iter()
            .find(|row| row.sequence == root && row.destination == message.destination)
        else {
            return false;
        };
        if !visible(parent, agent_id) {
            return false;
        }
        thread = parent.thread;
    }
    thread.is_none()
}
/// A private root's sender participates alongside its explicitly named readers.
pub(super) fn thread_readers(
    state: &StoredState,
    destination: &Destination,
    thread: Option<u64>,
) -> Result<Vec<String>> {
    let Some(root) = thread else {
        return Ok(Vec::new());
    };
    let parent = state
        .messages
        .iter()
        .find(|row| row.sequence == root && &row.destination == destination)
        .ok_or(Error::InvalidThread(root))?;
    let inherited = thread_readers_ancestors(state, parent)?;
    Ok(inherited)
}
fn thread_readers_ancestors(state: &StoredState, message: &Message) -> Result<Vec<String>> {
    let mut readers = Vec::new();
    let mut parent = message;
    for _ in 0..=state.messages.len() {
        if !parent.only_for.is_empty() {
            let mut scope = parent.only_for.clone();
            if !scope.contains(&parent.sender) {
                scope.push(parent.sender.clone());
            }
            if readers.is_empty() {
                readers = scope;
            } else {
                readers.retain(|reader| scope.contains(reader));
            }
        }
        let Some(root) = parent.thread else {
            return Ok(readers);
        };
        parent = state
            .messages
            .iter()
            .find(|row| row.sequence == root && row.destination == message.destination)
            .ok_or(Error::InvalidThread(root))?;
    }
    Err(Error::InvalidThread(message.sequence))
}
pub(super) fn narrow_readers(
    state: &StoredState,
    destination: &Destination,
    thread: Option<u64>,
    mut readers: Vec<String>,
) -> Result<Vec<String>> {
    let scope = thread_readers(state, destination, thread)?;
    if scope.is_empty() {
        return Ok(readers);
    }
    if readers.is_empty() {
        return Ok(scope);
    }
    readers.retain(|reader| scope.contains(reader));
    if readers.is_empty() {
        return Err(Error::InvalidThread(thread.unwrap_or_default()));
    }
    Ok(readers)
}

/// Retry authority is part of the same atomic identity as the authored payload.
fn retry_receipt(
    state: &StoredState,
    request: &SendMessage,
    scheduled_job_id: Option<&str>,
) -> Result<Option<Receipt>> {
    let Some(old) = state.accepted.get(&request.message_id) else {
        return Ok(None);
    };
    if old != request {
        return Err(Error::MessageConflict(request.message_id.clone()));
    }
    let message = state
        .messages
        .iter()
        .find(|msg| msg.message_id == request.message_id)
        .ok_or_else(|| Error::InvalidState("accepted message missing transcript row".into()))?;
    if message.scheduled_job_id.as_deref() != scheduled_job_id {
        return Err(Error::MessageConflict(request.message_id.clone()));
    }
    Ok(Some(Receipt {
        message_id: request.message_id.clone(),
        sequence: message.sequence,
    }))
}
