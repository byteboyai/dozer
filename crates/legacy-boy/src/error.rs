use thiserror::Error;

#[derive(Debug, Error)]
pub enum ShyError {
    #[error("model '{0}' not found in config")]
    ModelNotFound(String),
    #[error("agent '{0}' not found in config")]
    AgentNotFound(String),
    #[error("model '{0}' is not running")]
    ProcessNotRunning(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_not_found_message() {
        let e = ShyError::ModelNotFound("qwen36".into());
        assert_eq!(e.to_string(), "model 'qwen36' not found in config");
    }
}
