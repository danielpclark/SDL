// Part of the Rust translation of SPIRV-Cross; see LICENSE.txt.

//! CompilerGLSL (spirv_glsl.cpp). The translation lands in a later commit.

use super::common::*;
use super::cross::Compiler;

#[derive(Debug, Default)]
pub struct GlslState {}

impl Compiler {
    pub(crate) fn glsl_init(&mut self) -> Result<()> {
        Ok(())
    }
    pub(crate) fn glsl_compile(&mut self) -> Result<String> {
        spirv_cross_throw!("GLSL backend not translated yet.")
    }
}
