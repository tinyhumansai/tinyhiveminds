//! Live TypeSafe transport and routing request construction.

use tinyhivemind_core::typesafe::{
    Error, SystemOneRequest, SystemOneResponse, SystemOneTransport, SystemOneTransportFuture,
};

#[derive(Clone, Debug)]
pub(super) struct Transport {
    client: reqwest::Client,
    api_key: String,
}

impl Transport {
    pub(super) fn new(api_key: String) -> Result<Self, reqwest::Error> {
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            api_key,
        })
    }
}

impl SystemOneTransport for Transport {
    fn evaluate<'a>(&'a self, request: &'a SystemOneRequest) -> SystemOneTransportFuture<'a> {
        Box::pin(async move {
            let response = self
                .client
                .post("https://api.typesafe.ai/v1/systemone")
                .bearer_auth(&self.api_key)
                .json(request)
                .send()
                .await
                .map_err(|error| Error::Transport {
                    status: None,
                    message: error.to_string(),
                })?;
            let status = response.status();
            if !status.is_success() {
                let message = response.text().await.unwrap_or_default();
                return Err(Error::Transport {
                    status: Some(status.as_u16()),
                    message,
                });
            }
            response
                .json::<SystemOneResponse>()
                .await
                .map_err(|error| Error::Transport {
                    status: Some(status.as_u16()),
                    message: error.to_string(),
                })
        })
    }
}

pub(super) fn thread_context() -> Vec<String> {
    vec!["Only explicit completion ends an assignment".into()]
}
