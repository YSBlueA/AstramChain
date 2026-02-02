use crate::p2p::messages::HandshakeInfo;
use bytes::BytesMut;
use futures::SinkExt;
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::TcpStream;
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

pub type PeerId = String;

pub struct Peer {
    pub id: PeerId,
    pub reader: FramedRead<ReadHalf<TcpStream>, LengthDelimitedCodec>,
    pub writer: FramedWrite<WriteHalf<TcpStream>, LengthDelimitedCodec>,
    pub handshake_info: Option<HandshakeInfo>,
}

impl Peer {
    pub fn new(id: PeerId, stream: TcpStream) -> Self {
        let (read_half, write_half) = tokio::io::split(stream);
        let reader = FramedRead::new(read_half, LengthDelimitedCodec::new());
        let writer = FramedWrite::new(write_half, LengthDelimitedCodec::new());
        Self {
            id,
            reader,
            writer,
            handshake_info: None,
        }
    }

    pub async fn next_bytes(&mut self) -> anyhow::Result<Option<BytesMut>> {
        use tokio_stream::StreamExt;
        match self.reader.next().await {
            Some(Ok(bytes)) => Ok(Some(bytes)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    pub async fn send_bytes(&mut self, data: BytesMut) -> anyhow::Result<()> {
        // LengthDelimitedCodec expects bytes::Bytes, so freeze the BytesMut first.
        self.writer.send(data.freeze()).await.map_err(|e| e.into())
    }
}
