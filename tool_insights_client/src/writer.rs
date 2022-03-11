use crate::message::Message;

pub trait Writer {
    fn write(&self, messages: &[Message]) -> anyhow::Result<()>;
}
