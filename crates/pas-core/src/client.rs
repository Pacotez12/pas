use std::collections::VecDeque;
use std::io::ErrorKind;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::UnixStream;

use crate::CoreError;
use crate::protocol::{Notification, Request, Response};
use crate::socket::{validate_socket_path, write_msg};

#[derive(Debug)]
pub struct Client {
    stream: BufReader<UnixStream>,
    // Vec + read_until: cancel-safe in tokio::select! (read_line is not).
    read_buf: Vec<u8>,
    next_id: u64,
    pending_events: VecDeque<Notification>,
}

impl Client {
    pub async fn connect(path: &Path) -> Result<Self, CoreError> {
        validate_socket_path(path)?;
        let stream = UnixStream::connect(path)
            .await
            .map_err(|e| match e.kind() {
                ErrorKind::NotFound | ErrorKind::ConnectionRefused => CoreError::NotRunning,
                _ => CoreError::Io(e),
            })?;
        Ok(Self {
            stream: BufReader::new(stream),
            read_buf: Vec::new(),
            next_id: 1,
            pending_events: VecDeque::new(),
        })
    }

    async fn read_value(&mut self) -> Result<Option<Value>, CoreError> {
        let n = self.stream.read_until(b'\n', &mut self.read_buf).await?;
        if n == 0 && self.read_buf.is_empty() {
            return Ok(None);
        }
        let line = std::mem::take(&mut self.read_buf);
        Ok(Some(serde_json::from_slice(&line)?))
    }

    pub async fn call<P: Serialize, R: DeserializeOwned>(
        &mut self,
        method: &str,
        params: P,
    ) -> Result<R, CoreError> {
        let id = self.next_id;
        self.next_id += 1;

        let req = Request {
            jsonrpc: "2.0".into(),
            id,
            method: method.into(),
            params: serde_json::to_value(params)?,
        };
        write_msg(&mut self.stream, &req).await?;

        loop {
            let msg: Value = self.read_value().await?.ok_or_else(|| {
                CoreError::Io(std::io::Error::new(
                    ErrorKind::UnexpectedEof,
                    "pasd cerró la conexión antes de responder. Reintentá la operación.",
                ))
            })?;

            if !has_id(&msg) {
                let notif: Notification = serde_json::from_value(msg)?;
                self.pending_events.push_back(notif);
                continue;
            }

            let resp: Response = serde_json::from_value(msg)?;
            if resp.id != id {
                return Err(CoreError::Io(std::io::Error::new(
                    ErrorKind::InvalidData,
                    "pasd devolvió una respuesta fuera de orden. Reconectá e intentá de nuevo.",
                )));
            }

            if let Some(err) = resp.error {
                return Err(CoreError::Rpc {
                    code: err.code,
                    message: err.message,
                });
            }

            let value = resp.result.unwrap_or(Value::Null);
            return Ok(serde_json::from_value(value)?);
        }
    }

    pub async fn next_event(&mut self) -> Result<Option<Notification>, CoreError> {
        if let Some(event) = self.pending_events.pop_front() {
            return Ok(Some(event));
        }

        loop {
            let Some(msg) = self.read_value().await? else {
                return Ok(None);
            };
            if !has_id(&msg) {
                return Ok(Some(serde_json::from_value(msg)?));
            }
        }
    }
}

fn has_id(msg: &Value) -> bool {
    msg.get("id").is_some_and(|v| !v.is_null())
}
