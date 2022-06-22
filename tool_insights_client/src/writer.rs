// Copyright 2022 Twitter, Inc.
// SPDX-License-Identifier: Apache-2.0

use crate::message::Message;

pub trait Writer {
    fn write(&self, messages: &[Message]) -> anyhow::Result<()>;
}
