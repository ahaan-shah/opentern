//! `request.security` (Phase 10).

use crate::compile::Sec;
use crate::run::{Frame, Stop, X};
use crate::value::V;

#[derive(Clone, Debug)]
pub(crate) struct SecState;

impl SecState {
    pub fn refs(&self, _: &mut Vec<u32>) {}
}

pub(crate) fn security(_: &mut X, _: &Sec, _: &mut Frame, _: Option<&Frame>) -> Result<V, Stop> {
    Err(Stop::Err("request.security isn't supported yet.".into()))
}
