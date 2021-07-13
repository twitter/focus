use anyhow::{bail, Context, Result};
use std::{
    fs::File,
    io::{Read, Write},
};

struct ProtoFileStream {
    file: File,
}

const U32_BYTE_LENGTH: u32 = 32 / 8;

impl ProtoFileStream {
    pub fn new(file: File) -> Result<Self> {
        Ok(Self { file })
    }

    pub fn read<T: protobuf::Message>(&mut self) -> Result<(T, u32)> {
        let mut size_bytes: [u8; U32_BYTE_LENGTH as usize] = [0; U32_BYTE_LENGTH as usize];

        self.file
            .read_exact(&mut size_bytes)
            .context("reading message length")?;
        let size = u32::from_be_bytes(size_bytes);
        let mut message_bytes = Vec::<u8>::with_capacity(size as usize);
        self.file
            .read_exact(message_bytes.as_mut_slice())
            .context("reading message body")?;

        Ok((
            T::parse_from_bytes(message_bytes.as_slice()).context("parsing message")?,
            size + U32_BYTE_LENGTH,
        ))
    }

    pub fn write<T>(&mut self, t: &T) -> Result<u32>
    where
        T: protobuf::Message,
    {
        let size = t.compute_size();
        let size_write_result = self
            .file
            .write(&size.to_be_bytes()[..])
            .context("writing message length");
        match size_write_result {
            Ok(len) => {
                if len != U32_BYTE_LENGTH as usize {
                    bail!("wrote the wrong number of bytes while writing message length");
                }
            }
            Err(e) => {
                bail!("failed to write message length: {}", e)
            }
        }

        t.write_to_writer(&mut self.file)
            .context("writing message body")?;

        Ok(size + U32_BYTE_LENGTH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn writing_and_reading() -> Result<()> {
        let file = tempfile::tempfile()?;

        let model = focus_formats::testing::Foo { bar: 128 };

        // let mut stream = ProtoFileStream::new(file);
        // stream.wr()
        Ok(())
    }
}
