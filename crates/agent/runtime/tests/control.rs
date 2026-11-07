//! Offline controlled model regression suite.
use serde_json::Value;
use stkcraft_agent_runtime::{
    CancellationToken, ChatModel, Message, ModelError, RequestControl, Turn,
};
struct Poison;
impl ChatModel for Poison {
    fn complete(&self, _: &[Message], _: &[Value]) -> Result<Turn, String> {
        Err("secret poison".into())
    }
}
#[test]
fn cancellation_and_legacy_redaction() {
    let token = CancellationToken::default();
    let control = RequestControl::new(token.clone());
    assert_eq!(
        Poison.complete_controlled(&[], &[], &control),
        Err(ModelError::ModelFailed)
    );
    token.cancel();
    assert_eq!(control.check(), Err(ModelError::Cancelled));
}
#[test]
fn zero_timeout_is_rejected() {
    assert!(matches!(
        RequestControl::with_timeout(CancellationToken::default(), std::time::Duration::ZERO),
        Err(ModelError::Configuration)
    ));
}
