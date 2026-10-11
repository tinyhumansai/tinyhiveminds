//! Argument rules use authenticated context and refuse unknown required metadata.
use super::EffectClassifier;
use crate::TurnScope;
use openhuman_embed::seams::ToolHookContext;
use tinytools::{ApprovalDirective, RuleContext, Surface, ToolMatcher, ToolRules, ToolSubject};
pub(super) fn gate<'a>(
    rules: impl Iterator<Item = &'a ToolRules>,
    context: &ToolHookContext,
    seat_id: &str,
    scope: Option<&TurnScope>,
    classifier: Option<&dyn EffectClassifier>,
) -> Result<bool, &'static str> {
    let mut subject = classifier
        .and_then(|classifier| classifier.subject(context))
        .unwrap_or_else(|| ToolSubject::named(&context.tool_name));
    // A host can supply metadata, never replace the authenticated tool name.
    subject.name.clone_from(&context.tool_name);
    let mut attributes = classifier.map_or_else(RuleContext::new, |classifier| {
        classifier.rule_context(context)
    });
    attributes
        .attributes
        .insert("agent_id".into(), seat_id.into());
    attributes.attributes.remove("hive_id");
    attributes.attributes.remove("episode_id");
    attributes.attributes.remove("job_id");
    if let Some(session) = &context.session_id {
        attributes
            .attributes
            .insert("session_id".into(), session.clone());
    } else {
        attributes.attributes.remove("session_id");
    }
    if let Some(scope) = scope {
        if let Some(episode) = &scope.episode {
            attributes
                .attributes
                .insert("hive_id".into(), episode.hive_id.clone());
            attributes
                .attributes
                .insert("episode_id".into(), episode.episode_id.clone());
        }
        if let Some(job) = &scope.scheduled_job_id {
            attributes.attributes.insert("job_id".into(), job.clone());
        }
    }
    let mut ask = false;
    for rules in rules {
        for rule in &rules.rules {
            if !rule.on.is_empty() && !rule.on.contains(&Surface::Call) {
                continue;
            }
            if unknown(&rule.matcher, &subject)
                || rule
                    .except
                    .as_ref()
                    .is_some_and(|matcher| unknown(matcher, &subject))
                || rule.when.keys().any(|key| attributes.get(key).is_none())
            {
                return Err("tool rule requires unknown metadata");
            }
        }
        let verdict = rules.evaluate(
            &subject,
            &attributes,
            Surface::Call,
            Some(&context.arguments),
        );
        if !verdict.callable {
            return Err("tool rule denied");
        }
        ask |= verdict.approval == ApprovalDirective::Required;
    }
    Ok(ask)
}
fn unknown(matcher: &ToolMatcher, subject: &ToolSubject) -> bool {
    matcher.family.is_some() && subject.family.is_none()
        || matcher.tags.is_some() && subject.tags.is_empty()
        || matcher.category.is_some() && subject.category.is_none()
        || matcher.exposure.is_some() && subject.exposure.is_none()
        || (matcher.permission_at_least.is_some() || matcher.permission_at_most.is_some())
            && subject.permission.is_none()
        || matcher.side_effects.is_some() && subject.side_effects.is_none()
}
