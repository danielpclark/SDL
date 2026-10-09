// Part of the Rust translation of SPIRV-Cross; see LICENSE.txt.

//! CompilerMSL (spirv_msl.cpp). The translation lands in a later commit.

use super::common::*;
use super::cross::Compiler;

#[derive(Default)]
pub struct MslState {}

impl Compiler {
    pub(crate) fn msl_init(&mut self) -> Result<()> {
        Ok(())
    }
    pub(crate) fn msl_compile(&mut self) -> Result<String> {
        Ok(String::new())
    }
    pub(crate) fn msl_to_name(&self, id: u32, allow_alias: bool) -> Result<String> {
        self.base_to_name(id, allow_alias)
    }
}
