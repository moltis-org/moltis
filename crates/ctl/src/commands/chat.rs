//! Chat subcommands.

use {
    crate::client::CtlClient,
    serde_json::{Value, json},
};

pub async fn send(client: &mut CtlClient, text: &str, session_key: &str) -> anyhow::Result<Value> {
    client
        .call(
            "chat.send",
            json!({
                "text": text,
                "_session_key": session_key,
            }),
        )
        .await
        .map_err(Into::into)
}

pub async fn history(client: &mut CtlClient, session_key: &str) -> anyhow::Result<Value> {
    client
        .call(
            "chat.history",
            json!({
                "_session_key": session_key,
            }),
        )
        .await
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn send_params_preserve_terminal_bench_session() {
        let params = json!({
            "text": "fix the task",
            "_session_key": "terminal-bench:task-1",
        });

        assert_eq!(params["text"], "fix the task");
        assert_eq!(params["_session_key"], "terminal-bench:task-1");
    }
}
