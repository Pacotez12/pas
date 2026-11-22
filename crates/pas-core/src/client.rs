use std::io::ErrorKind;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::BufReader;
use tokio::net::UnixStream;

use crate::CoreError;
use crate::protocol::{Request, Response};
use crate::socket::{read_msg, validate_socket_path, write_msg};

#[derive(Debug)]
pub struct Client {
    stream: BufReader<UnixStream>,
    next_id: u64,
}

impl Client {
    pub async fn connect(path: &Path) -> Result<Self, CoreError> {
        validate_socket_path(path)?;
        let stream = UnixStream::connect(path).await.map_err(|e| match e.kind() {
            ErrorKind::NotFound | ErrorKind::ConnectionRefused => CoreError::NotRunning,
            _ => CoreError::Io(e),
        })?;
        Ok(Self {
            stream: BufReader::new(stream),
            next_id: 1,
        })
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

        let resp: Response = read_msg(&mut self.stream).await?.ok_or_else(|| {
            CoreError::Io(std::io::Error::new(
                ErrorKind::UnexpectedEof,
                "pasd cerró la conexión antes de responder.",
            ))
        })?;

        if let Some(err) = resp.error {
            return Err(CoreError::Rpc {
                code: err.code,
                message: err.message,
            });
        }

        let value = resp.result.unwrap_or(serde_json::Value::Null);
        Ok(serde_json::from_value(value)?)
    }
}
