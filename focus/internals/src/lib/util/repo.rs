use std::{path::Path, str::FromStr, sync::Arc};

use anyhow::{bail, Context, Result};
use uuid::Uuid;

use crate::{app::App, ui::ProgressReporter, util::git_helper};

