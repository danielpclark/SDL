// Rust translation of src/format.c and src/format.h from libxmp 4.7.3 (as vendored by SDL_mixer).
// Extended Module Player
// Copyright (C) 1996-2024 Claudio Matsuoka and Hipolito Carraro Jr
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The format loaders, in the order they're tried. (The ProWizard
//! loader that ends upstream's list is compiled out by SDL_mixer's build.)

use super::loaders::abk_load::LIBXMP_LOADER_ABK;
use super::loaders::amf_load::LIBXMP_LOADER_AMF;
use super::loaders::arch_load::LIBXMP_LOADER_ARCH;
use super::loaders::asylum_load::LIBXMP_LOADER_ASYLUM;
use super::loaders::c669_load::LIBXMP_LOADER_669;
use super::loaders::chip_load::LIBXMP_LOADER_CHIP;
use super::loaders::coco_load::LIBXMP_LOADER_COCO;
use super::loaders::dbm_load::LIBXMP_LOADER_DBM;
use super::loaders::digi_load::LIBXMP_LOADER_DIGI;
use super::loaders::dt_load::LIBXMP_LOADER_DT;
use super::loaders::emod_load::LIBXMP_LOADER_EMOD;
use super::loaders::far_load::LIBXMP_LOADER_FAR;
use super::loaders::flt_load::LIBXMP_LOADER_FLT;
use super::loaders::fnk_load::LIBXMP_LOADER_FNK;
use super::loaders::gal4_load::LIBXMP_LOADER_GAL4;
use super::loaders::gal5_load::LIBXMP_LOADER_GAL5;
use super::loaders::gdm_load::LIBXMP_LOADER_GDM;
use super::loaders::hmn_load::LIBXMP_LOADER_HMN;
use super::loaders::ice_load::LIBXMP_LOADER_ICE;
use super::loaders::imf_load::LIBXMP_LOADER_IMF;
use super::loaders::ims_load::LIBXMP_LOADER_IMS;
use super::loaders::it_load::LIBXMP_LOADER_IT;
use super::loaders::liq_load::LIBXMP_LOADER_LIQ;
use super::loaders::masi16_load::LIBXMP_LOADER_MASI16;
use super::loaders::masi_load::LIBXMP_LOADER_MASI;
use super::loaders::mdl_load::LIBXMP_LOADER_MDL;
use super::loaders::med2_load::LIBXMP_LOADER_MED2;
use super::loaders::med3_load::LIBXMP_LOADER_MED3;
use super::loaders::med4_load::LIBXMP_LOADER_MED4;
use super::loaders::mfp_load::LIBXMP_LOADER_MFP;
use super::loaders::mgt_load::LIBXMP_LOADER_MGT;
use super::loaders::mmd1_load::LIBXMP_LOADER_MMD1;
use super::loaders::mmd3_load::LIBXMP_LOADER_MMD3;
use super::loaders::mod_load::LIBXMP_LOADER_MOD;
use super::loaders::mtm_load::LIBXMP_LOADER_MTM;
use super::loaders::muse_load::LIBXMP_LOADER_MUSE;
use super::loaders::no_load::LIBXMP_LOADER_NO;
use super::loaders::okt_load::LIBXMP_LOADER_OKT;
use super::loaders::pt3_load::LIBXMP_LOADER_PT3;
use super::loaders::ptm_load::LIBXMP_LOADER_PTM;
use super::loaders::rtm_load::LIBXMP_LOADER_RTM;
use super::loaders::s3m_load::LIBXMP_LOADER_S3M;
use super::loaders::sfx_load::LIBXMP_LOADER_SFX;
use super::loaders::st_load::LIBXMP_LOADER_ST;
use super::loaders::stim_load::LIBXMP_LOADER_STIM;
use super::loaders::stm_load::LIBXMP_LOADER_STM;
use super::loaders::stx_load::LIBXMP_LOADER_STX;
use super::loaders::sym_load::LIBXMP_LOADER_SYM;
use super::loaders::ult_load::LIBXMP_LOADER_ULT;
use super::loaders::umx_load::LIBXMP_LOADER_UMX;
use super::loaders::xm_load::LIBXMP_LOADER_XM;
use super::loaders::xmf_load::LIBXMP_LOADER_XMF;
use super::loaders::FormatLoader;

/// Translation of `format_loaders`.
pub(crate) static FORMAT_LOADERS: &[&FormatLoader] = &[
    &LIBXMP_LOADER_XM,
    &LIBXMP_LOADER_MOD,
    &LIBXMP_LOADER_IT,
    &LIBXMP_LOADER_S3M,
    &LIBXMP_LOADER_FLT,
    &LIBXMP_LOADER_ST,
    &LIBXMP_LOADER_STM,
    &LIBXMP_LOADER_STX,
    &LIBXMP_LOADER_MTM,
    &LIBXMP_LOADER_ICE,
    &LIBXMP_LOADER_IMF,
    &LIBXMP_LOADER_PTM,
    &LIBXMP_LOADER_MDL,
    &LIBXMP_LOADER_ULT,
    &LIBXMP_LOADER_LIQ,
    &LIBXMP_LOADER_NO,
    &LIBXMP_LOADER_MASI,
    &LIBXMP_LOADER_MASI16,
    &LIBXMP_LOADER_MUSE,
    &LIBXMP_LOADER_GAL5,
    &LIBXMP_LOADER_GAL4,
    &LIBXMP_LOADER_AMF,
    &LIBXMP_LOADER_ASYLUM,
    &LIBXMP_LOADER_GDM,
    &LIBXMP_LOADER_MMD1,
    &LIBXMP_LOADER_MMD3,
    &LIBXMP_LOADER_MED2,
    &LIBXMP_LOADER_MED3,
    &LIBXMP_LOADER_MED4,
    &LIBXMP_LOADER_CHIP,
    &LIBXMP_LOADER_RTM,
    &LIBXMP_LOADER_PT3,
    &LIBXMP_LOADER_DT,
    &LIBXMP_LOADER_MGT,
    &LIBXMP_LOADER_ARCH,
    &LIBXMP_LOADER_SYM,
    &LIBXMP_LOADER_DIGI,
    &LIBXMP_LOADER_DBM,
    &LIBXMP_LOADER_EMOD,
    &LIBXMP_LOADER_OKT,
    &LIBXMP_LOADER_SFX,
    &LIBXMP_LOADER_FAR,
    &LIBXMP_LOADER_UMX,
    &LIBXMP_LOADER_HMN,
    &LIBXMP_LOADER_STIM,
    &LIBXMP_LOADER_COCO,
    &LIBXMP_LOADER_IMS,
    &LIBXMP_LOADER_669,
    &LIBXMP_LOADER_FNK,
    &LIBXMP_LOADER_MFP,
    &LIBXMP_LOADER_ABK,
    &LIBXMP_LOADER_XMF,
];
