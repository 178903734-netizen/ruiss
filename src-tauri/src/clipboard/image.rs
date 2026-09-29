//! Bounded image streaming keeps PNG bytes out of JSON and lets control messages
//! run between chunks. File-copy offers continue to use the file transfer engine.
use crate::core::protocol::{Message, Payload};
use crate::net::NetHandle;

const CHUNK_SIZE: usize = 256 * 1024;
const MAX_IMAGE_SIZE: usize = 256 * 1024 * 1024;

pub fn send(net: &NetHandle, name: &str, id: String, png: Vec<u8>) {
    if png.is_empty() || png.len() > MAX_IMAGE_SIZE {
        log::error!(
            "[CLIPBOARD] image size {} is outside 1..={MAX_IMAGE_SIZE}",
            png.len()
        );
        return;
    }
    // Announce synchronously so a later physical copy always supersedes this image.
    net.send_ctrl(Message::clipboard(
        name,
        Payload::ClipboardImageStart {
            id: id.clone(),
            size: png.len() as u64,
        },
    ));
    let net = net.clone();
    let name = name.to_owned();
    tauri::async_runtime::spawn(async move {
        for (seq, data) in png.chunks(CHUNK_SIZE).enumerate() {
            if let Err(error) = net
                .send_file(Message::clipboard(
                    &name,
                    Payload::ClipboardImageChunk {
                        id: id.clone(),
                        seq: seq as u32,
                        data: data.to_vec(),
                    },
                ))
                .await
            {
                log::error!("[CLIPBOARD] image transfer failed: {error}");
                return;
            }
        }
        if let Err(error) = net
            .send_file(Message::clipboard(&name, Payload::ClipboardImageEnd { id }))
            .await
        {
            log::error!("[CLIPBOARD] image completion failed: {error}");
        }
    });
}

#[derive(Default)]
pub struct ImageReceiver {
    pending: Option<PendingImage>,
}

struct PendingImage {
    id: String,
    size: usize,
    seq: u32,
    data: Vec<u8>,
}

impl ImageReceiver {
    pub fn clear(&mut self) {
        self.pending = None;
    }

    pub fn start(&mut self, id: &str, size: u64) -> Result<(), &'static str> {
        self.clear();
        if id.is_empty() || size == 0 || size > MAX_IMAGE_SIZE as u64 {
            return Err("invalid clipboard image size or id");
        }
        self.pending = Some(PendingImage {
            id: id.into(),
            size: size as usize,
            seq: 0,
            data: Vec::new(),
        });
        Ok(())
    }

    pub fn chunk(&mut self, id: &str, seq: u32, data: &[u8]) -> Result<(), &'static str> {
        let Some(pending) = &mut self.pending else {
            return Ok(());
        };
        if pending.id != id {
            return Ok(());
        }
        if seq != pending.seq
            || data.is_empty()
            || data.len() > CHUNK_SIZE
            || data.len() > pending.size.saturating_sub(pending.data.len())
        {
            self.clear();
            return Err("invalid clipboard image chunk");
        }
        pending.data.extend_from_slice(data);
        pending.seq += 1;
        Ok(())
    }

    pub fn finish(&mut self, id: &str) -> Option<Vec<u8>> {
        if self.pending.as_ref()?.id != id {
            return None;
        }
        let pending = self.pending.take()?;
        (pending.data.len() == pending.size).then_some(pending.data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_larger_than_old_frame_limit_round_trips() {
        let bytes: Vec<u8> = (0..20 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let mut receiver = ImageReceiver::default();
        receiver.start("large", bytes.len() as u64).unwrap();
        for (seq, chunk) in bytes.chunks(CHUNK_SIZE).enumerate() {
            receiver.chunk("large", seq as u32, chunk).unwrap();
        }
        assert_eq!(receiver.finish("large").unwrap(), bytes);
    }

    #[test]
    fn rejects_missing_out_of_order_and_oversized_data() {
        let mut receiver = ImageReceiver::default();
        assert!(receiver.start("x", MAX_IMAGE_SIZE as u64 + 1).is_err());
        receiver.start("x", 4).unwrap();
        receiver.chunk("x", 0, &[1, 2]).unwrap();
        assert!(receiver.finish("x").is_none());
        receiver.start("x", 4).unwrap();
        assert!(receiver.chunk("x", 1, &[1, 2]).is_err());
        assert!(receiver.finish("x").is_none());
        receiver.start("x", 1).unwrap();
        assert!(receiver.chunk("x", 0, &[1, 2]).is_err());
    }

    #[test]
    fn superseded_image_cannot_overwrite_new_copy() {
        let mut receiver = ImageReceiver::default();
        receiver.start("old", 1).unwrap();
        receiver.start("new", 1).unwrap();
        receiver.chunk("old", 0, &[7]).unwrap();
        assert!(receiver.finish("old").is_none());
        receiver.chunk("new", 0, &[8]).unwrap();
        assert_eq!(receiver.finish("new"), Some(vec![8]));
    }
}
