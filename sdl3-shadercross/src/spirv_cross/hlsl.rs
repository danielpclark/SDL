// Part of the Rust translation of SPIRV-Cross; see LICENSE.txt.

//! CompilerHLSL (spirv_hlsl.cpp). The translation lands in a later commit.

use super::common::*;
use super::cross::Compiler;

#[derive(Default)]
pub struct HlslState {}

impl Compiler {
    pub(crate) fn hlsl_init(&mut self) -> Result<()> {
        Ok(())
    }
    pub(crate) fn hlsl_compile(&mut self) -> Result<String> {
        Ok(String::new())
    }
}
