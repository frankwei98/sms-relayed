use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;

use crate::config::{AppConfig, ChannelProfile};

pub(super) use crate::forward::ForwardOutcome as DispatchOutcome;

pub(super) type DispatchFuture<'a> = Pin<Box<dyn Future<Output = DispatchResult> + Send + 'a>>;

pub(super) trait Dispatcher: Send + Sync {
    fn dispatch<'a>(&'a self, request: DispatchRequest<'a>) -> DispatchFuture<'a>;
}

pub(super) struct DispatchRequest<'a> {
    pub profile_key: &'a str,
    pub phone_number: &'a str,
    pub body: &'a str,
    pub timestamp: &'a str,
}

pub(super) enum DispatchResult {
    ProfileMissing,
    Attempted(DispatchOutcome),
}

pub(super) struct ProductionDispatcher {
    config: AppConfig,
    profiles: Vec<ChannelProfile>,
    client: Arc<reqwest::Client>,
    webhook_client: Arc<reqwest::Client>,
}

impl ProductionDispatcher {
    pub(super) fn new(
        config: AppConfig,
        client: Arc<reqwest::Client>,
        webhook_client: Arc<reqwest::Client>,
    ) -> Result<Self> {
        let profiles = config.enabled_profiles()?;
        Ok(Self {
            config,
            profiles,
            client,
            webhook_client,
        })
    }
}

impl Dispatcher for ProductionDispatcher {
    fn dispatch<'a>(&'a self, request: DispatchRequest<'a>) -> DispatchFuture<'a> {
        let Some(profile) = self
            .profiles
            .iter()
            .find(|profile| profile.key() == request.profile_key)
        else {
            return Box::pin(async { DispatchResult::ProfileMissing });
        };
        Box::pin(async move {
            DispatchResult::Attempted(
                forward_to_profile(
                    &self.client,
                    &self.webhook_client,
                    profile,
                    request,
                    &self.config,
                )
                .await,
            )
        })
    }
}

async fn forward_to_profile(
    client: &reqwest::Client,
    webhook_client: &reqwest::Client,
    profile: &ChannelProfile,
    request: DispatchRequest<'_>,
    config: &AppConfig,
) -> DispatchOutcome {
    let DispatchRequest {
        phone_number: tel_number,
        body,
        timestamp,
        ..
    } = request;
    let device_name =
        if config.app.device_name == "*Host*Name*" || config.app.device_name.is_empty() {
            crate::util::hostname()
        } else {
            config.app.device_name.clone()
        };

    match profile {
        ChannelProfile::Bark { config: pc, .. } => {
            crate::forward::bark::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        ChannelProfile::Telegram { config: pc, .. } => {
            crate::forward::telegram::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        ChannelProfile::WeCom { config: pc, .. } => {
            crate::forward::wecom::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        ChannelProfile::DingTalk { config: pc, .. } => {
            crate::forward::dingtalk::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        ChannelProfile::Lark { config: pc, .. } => {
            crate::forward::lark::send(
                client,
                tel_number,
                body,
                timestamp,
                &device_name,
                pc,
                config,
            )
            .await
        }
        ChannelProfile::Webhook { config: pc, .. } => {
            crate::forward::webhook::send(
                webhook_client,
                crate::forward::webhook::WebhookMessage {
                    sender: tel_number,
                    message: body,
                    datetime: timestamp,
                },
                pc,
            )
            .await
        }
    }
}

#[cfg(test)]
mod scripted {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;
    use std::time::Duration;

    use tokio::sync::Notify;

    use super::{DispatchFuture, DispatchOutcome, DispatchRequest, DispatchResult, Dispatcher};

    pub(in crate::delivery) enum ScriptedAction {
        Success,
        SuccessAfter(Duration),
        TransientFailure(String),
        TransientFailureAfter(Duration, String),
        PermanentFailure(String),
        ProfileMissing,
        Hang,
    }

    pub(in crate::delivery) struct ScriptedDispatcher {
        actions: Mutex<VecDeque<ScriptedAction>>,
        requests: Mutex<Vec<CapturedRequest>>,
        calls: AtomicUsize,
        completions: AtomicUsize,
        active: AtomicUsize,
        max_active: AtomicUsize,
        cancellations: AtomicUsize,
        changed: Notify,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub(in crate::delivery) struct CapturedRequest {
        pub(in crate::delivery) profile_key: String,
        pub(in crate::delivery) phone_number: String,
        pub(in crate::delivery) body: String,
        pub(in crate::delivery) timestamp: String,
    }

    impl ScriptedDispatcher {
        pub(in crate::delivery) fn new(actions: impl IntoIterator<Item = ScriptedAction>) -> Self {
            Self {
                actions: Mutex::new(actions.into_iter().collect()),
                requests: Mutex::new(Vec::new()),
                calls: AtomicUsize::new(0),
                completions: AtomicUsize::new(0),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                cancellations: AtomicUsize::new(0),
                changed: Notify::new(),
            }
        }

        pub(in crate::delivery) async fn wait_for_calls(&self, expected: usize) {
            loop {
                let changed = self.changed.notified();
                if self.calls.load(Ordering::SeqCst) >= expected {
                    return;
                }
                changed.await;
            }
        }

        pub(in crate::delivery) async fn wait_for_completions(&self, expected: usize) {
            loop {
                let changed = self.changed.notified();
                if self.completions.load(Ordering::SeqCst) >= expected {
                    return;
                }
                changed.await;
            }
        }

        pub(in crate::delivery) fn requests(&self) -> Vec<CapturedRequest> {
            self.requests.lock().unwrap().clone()
        }

        pub(in crate::delivery) fn active(&self) -> usize {
            self.active.load(Ordering::SeqCst)
        }

        pub(in crate::delivery) fn max_active(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }

        pub(in crate::delivery) fn cancellations(&self) -> usize {
            self.cancellations.load(Ordering::SeqCst)
        }
    }

    impl Dispatcher for ScriptedDispatcher {
        fn dispatch<'a>(&'a self, request: DispatchRequest<'a>) -> DispatchFuture<'a> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(CapturedRequest {
                    profile_key: request.profile_key.to_string(),
                    phone_number: request.phone_number.to_string(),
                    body: request.body.to_string(),
                    timestamp: request.timestamp.to_string(),
                });
                let action = self
                    .actions
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("scripted dispatcher action queue exhausted");
                self.calls.fetch_add(1, Ordering::SeqCst);
                let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_active.fetch_max(active, Ordering::SeqCst);
                self.changed.notify_waiters();
                let mut guard = ActiveDispatch {
                    dispatcher: self,
                    completed: false,
                };

                let result = match action {
                    ScriptedAction::Success => DispatchResult::Attempted(DispatchOutcome::Success),
                    ScriptedAction::SuccessAfter(delay) => {
                        tokio::time::sleep(delay).await;
                        DispatchResult::Attempted(DispatchOutcome::Success)
                    }
                    ScriptedAction::TransientFailure(code) => {
                        DispatchResult::Attempted(DispatchOutcome::TransientFailure(code))
                    }
                    ScriptedAction::TransientFailureAfter(delay, code) => {
                        tokio::time::sleep(delay).await;
                        DispatchResult::Attempted(DispatchOutcome::TransientFailure(code))
                    }
                    ScriptedAction::PermanentFailure(code) => {
                        DispatchResult::Attempted(DispatchOutcome::PermanentFailure(code))
                    }
                    ScriptedAction::ProfileMissing => DispatchResult::ProfileMissing,
                    ScriptedAction::Hang => std::future::pending::<DispatchResult>().await,
                };

                guard.completed = true;
                self.completions.fetch_add(1, Ordering::SeqCst);
                self.changed.notify_waiters();
                result
            })
        }
    }

    struct ActiveDispatch<'a> {
        dispatcher: &'a ScriptedDispatcher,
        completed: bool,
    }

    impl Drop for ActiveDispatch<'_> {
        fn drop(&mut self) {
            self.dispatcher.active.fetch_sub(1, Ordering::SeqCst);
            if !self.completed {
                self.dispatcher.cancellations.fetch_add(1, Ordering::SeqCst);
            }
            self.dispatcher.changed.notify_waiters();
        }
    }
}

#[cfg(test)]
pub(super) use scripted::{ScriptedAction, ScriptedDispatcher};

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::config::AppConfig;

    use super::{
        DispatchRequest, DispatchResult, Dispatcher, ProductionDispatcher, ScriptedAction,
        ScriptedDispatcher,
    };

    #[tokio::test]
    async fn scripted_dispatcher_returns_the_next_action_and_captures_the_request() {
        let dispatcher = ScriptedDispatcher::new([ScriptedAction::Success]);

        let result = dispatcher
            .dispatch(DispatchRequest {
                profile_key: "bark.primary",
                phone_number: "+15550000000",
                body: "hello",
                timestamp: "2026-07-25T00:00:00Z",
            })
            .await;

        assert!(matches!(result, DispatchResult::Attempted(_)));
        assert_eq!(
            dispatcher.requests(),
            vec![super::scripted::CapturedRequest {
                profile_key: "bark.primary".to_string(),
                phone_number: "+15550000000".to_string(),
                body: "hello".to_string(),
                timestamp: "2026-07-25T00:00:00Z".to_string(),
            }]
        );
        assert_eq!(dispatcher.active(), 0);
        assert_eq!(dispatcher.max_active(), 1);
        assert_eq!(dispatcher.cancellations(), 0);
        dispatcher.wait_for_completions(1).await;
    }

    #[tokio::test]
    async fn production_dispatcher_reports_missing_profile_without_attempting() {
        let dispatcher = ProductionDispatcher::new(
            AppConfig::default(),
            Arc::new(reqwest::Client::new()),
            Arc::new(reqwest::Client::new()),
        )
        .unwrap();

        let result = dispatcher
            .dispatch(DispatchRequest {
                profile_key: "webhook.missing",
                phone_number: "+15550000000",
                body: "hello",
                timestamp: "2026-07-25T00:00:00Z",
            })
            .await;

        assert!(matches!(result, DispatchResult::ProfileMissing));
    }

    #[test]
    fn production_dispatcher_rejects_invalid_enabled_profile_at_construction() {
        let mut config = AppConfig::default();
        config.forward.enabled.push("bark.missing".to_string());

        let result = ProductionDispatcher::new(
            config,
            Arc::new(reqwest::Client::new()),
            Arc::new(reqwest::Client::new()),
        );

        assert!(result.is_err());
    }
}
