// Rust translation of the parts of av1/encoder/ml.c and av1/encoder/ml.h
// from aom (https://aomedia.googlesource.com/aom, at the revision SDL_image's
// external/aom pins: libaom 3.6.1 with SDL's patches) the saver reaches.
// Copyright (c) 2016, Alliance for Open Media. All rights reserved
// SPDX-License-Identifier: BSD-2-Clause, and the Alliance for Open Media
// Patent License 1.0 (see LICENSE.txt)
// This is an altered (translated) version of the original software; see LICENSE.txt.

//! The neural network predictor of the encoder's pruning models. The
//! arithmetic is single precision as upstream's (each operation rounded to
//! a float, as x86-64 builds of the C code evaluate them; the generic
//! target has no fused multiply-add to contract them into).

pub(crate) const NN_MAX_HIDDEN_LAYERS: usize = 10;
pub(crate) const NN_MAX_NODES_PER_LAYER: usize = 128;

/// Translation of `NN_CONFIG`.
pub(crate) struct NnConfig {
    /// Number of input nodes, i.e. features.
    pub(crate) num_inputs: i32,
    /// Number of output nodes.
    pub(crate) num_outputs: i32,
    /// Number of hidden layers, maximum 10.
    pub(crate) num_hidden_layers: i32,
    /// Number of nodes for each hidden layer.
    pub(crate) num_hidden_nodes: [i32; NN_MAX_HIDDEN_LAYERS],
    /// Weight parameters, indexed by layer.
    pub(crate) weights: [&'static [f32]; NN_MAX_HIDDEN_LAYERS + 1],
    /// Bias parameters, indexed by layer.
    pub(crate) bias: [&'static [f32]; NN_MAX_HIDDEN_LAYERS + 1],
}

/// Translation of `av1_nn_output_prec_reduce()`.
pub(crate) fn av1_nn_output_prec_reduce(output: &mut [f32], num_output: i32) {
    let prec_bits = 9;
    let prec = 1 << prec_bits;
    let inv_prec = (1.0 / prec as f64) as f32;
    for o in output.iter_mut().take(num_output as usize) {
        // (output[i] * prec is a float; adding the double 0.5 makes the sum a
        // double)
        *o = (((*o * prec as f32) as f64 + 0.5) as i32) as f32 * inv_prec;
    }
}

// Calculate prediction based on the given input features and neural net config.
// Assume there are no more than NN_MAX_NODES_PER_LAYER nodes in each hidden
// layer.
/// Translation of `av1_nn_predict_c()`.
pub(crate) fn av1_nn_predict(
    input: &[f32],
    nn_config: &NnConfig,
    reduce_prec: i32,
    output: &mut [f32],
) -> Result<(), ()> {
    let mut num_input_nodes = nn_config.num_inputs as usize;
    let mut buf = [[0f32; NN_MAX_NODES_PER_LAYER]; 2];
    let mut buf_index = 0;
    // (input_nodes: the input, then buf[1 - buf_index])
    let mut input_is_buf = false;

    // Propagate hidden layers.
    let num_layers = nn_config.num_hidden_layers as usize;
    if num_layers > NN_MAX_HIDDEN_LAYERS {
        return Err(());
    }
    for layer in 0..num_layers {
        let layer_weights = nn_config.weights[layer];
        let layer_bias = nn_config.bias[layer];
        let num_output_nodes = nn_config.num_hidden_nodes[layer] as usize;
        if num_output_nodes >= NN_MAX_NODES_PER_LAYER {
            return Err(());
        }
        let (b0, b1) = buf.split_at_mut(1);
        let (output_nodes, prev) = if buf_index == 0 {
            (&mut b0[0], &b1[0])
        } else {
            (&mut b1[0], &b0[0])
        };
        let input_nodes: &[f32] = if input_is_buf { prev } else { input };
        for node in 0..num_output_nodes {
            let mut val = layer_bias[node];
            for i in 0..num_input_nodes {
                val += layer_weights[node * num_input_nodes + i] * input_nodes[i];
            }
            // ReLU as activation function.
            val = if val > 0.0 { val } else { 0.0 }; // Could use AOMMAX().
            output_nodes[node] = val;
        }
        num_input_nodes = num_output_nodes;
        input_is_buf = true;
        buf_index = 1 - buf_index;
    }

    // Final output layer.
    let input_nodes: &[f32] = if input_is_buf {
        &buf[1 - buf_index]
    } else {
        input
    };
    let layer_weights = nn_config.weights[num_layers];
    let layer_bias = nn_config.bias[num_layers];
    for node in 0..nn_config.num_outputs as usize {
        let mut val = layer_bias[node];
        for i in 0..num_input_nodes {
            val += layer_weights[node * num_input_nodes + i] * input_nodes[i];
        }
        output[node] = val;
    }
    if reduce_prec != 0 {
        av1_nn_output_prec_reduce(output, nn_config.num_outputs);
    }
    Ok(())
}
