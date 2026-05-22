use anyhow::{Result, bail};
use iroh::endpoint::{RecvStream, SendStream};

/// Maximum size of a single framed IP packet. Generous upper bound; real
/// packets are capped by the TUN MTU which is well under this.
pub const MAX_FRAME: usize = 65535;

/// Writes a length-delimited frame: a big-endian u16 length followed by `data`.
pub async fn write_frame(send: &mut SendStream, data: &[u8]) -> Result<()> {
    if data.len() > MAX_FRAME {
        bail!("frame too large: {} bytes", data.len());
    }
    let len = (data.len() as u16).to_be_bytes();
    send.write_all(&len).await?;
    send.write_all(data).await?;
    Ok(())
}

/// Reads a single length-delimited frame into `buf`, returning its length.
///
/// Returns `Ok(None)` on a clean end of stream.
pub async fn read_frame(recv: &mut RecvStream, buf: &mut [u8]) -> Result<Option<usize>> {
    let mut len_buf = [0u8; 2];
    match recv.read_exact(&mut len_buf).await {
        Ok(()) => {}
        // Clean EOF before any byte of the next frame.
        Err(_) => return Ok(None),
    }
    let len = u16::from_be_bytes(len_buf) as usize;
    if len > buf.len() {
        bail!("frame length {} exceeds buffer {}", len, buf.len());
    }
    recv.read_exact(&mut buf[..len]).await?;
    Ok(Some(len))
}
