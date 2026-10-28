//! Recurrent layers, under PyTorch's and tch's names.

#![allow(clippy::upper_case_acronyms)]

use std::borrow::Borrow;

use tch::{Device, Result, Tensor};

pub trait RNN {
    type State;

    fn zero_state(&self, batch_dim: i64) -> Result<Self::State>;

    /// A whole sequence from a zero state.
    fn seq(&self, input: &Tensor) -> Result<(Tensor, Self::State)> {
        let state = self.zero_state(input.size()[0])?;
        self.seq_init(input, &state)
    }

    fn seq_init(&self, input: &Tensor, state: &Self::State) -> Result<(Tensor, Self::State)>;
}

/// An LSTM's hidden and cell states.
#[derive(Debug)]
pub struct LSTMState(pub (Tensor, Tensor));

#[derive(Debug, Clone, Copy)]
pub struct RNNConfig {
    pub has_biases: bool,
    pub num_layers: i64,
    pub dropout: f64,
    pub train: bool,
    pub bidirectional: bool,
    pub batch_first: bool,
}

impl Default for RNNConfig {
    fn default() -> Self {
        RNNConfig {
            has_biases: true,
            num_layers: 1,
            dropout: 0.,
            train: false,
            bidirectional: false,
            batch_first: true,
        }
    }
}

#[derive(Debug)]
pub struct LSTM {
    flat_weights: Vec<Tensor>,
    hidden_dim: i64,
    config: RNNConfig,
    device: Device,
}

/// An LSTM with PyTorch's weight names: `weight_ih_l0`, `bias_hh_l0_reverse`,
/// and so on.
pub fn lstm<'a, T: Borrow<super::Path<'a>>>(
    vs: T,
    in_dim: i64,
    hidden_dim: i64,
    c: RNNConfig,
) -> Result<LSTM> {
    let vs = vs.borrow();
    let directions = if c.bidirectional { 2 } else { 1 };
    let gate_dim = 4 * hidden_dim;
    let mut flat_weights = vec![];
    for layer in 0..c.num_layers {
        for direction in 0..directions {
            let in_dim = if layer == 0 {
                in_dim
            } else {
                hidden_dim * directions
            };
            let suffix = if direction == 1 { "_reverse" } else { "" };
            flat_weights.push(vs.var(&format!("weight_ih_l{layer}{suffix}"), &[gate_dim, in_dim])?);
            flat_weights.push(vs.var(
                &format!("weight_hh_l{layer}{suffix}"),
                &[gate_dim, hidden_dim],
            )?);
            if c.has_biases {
                flat_weights.push(vs.var(&format!("bias_ih_l{layer}{suffix}"), &[gate_dim])?);
                flat_weights.push(vs.var(&format!("bias_hh_l{layer}{suffix}"), &[gate_dim])?);
            }
        }
    }
    Ok(LSTM {
        flat_weights,
        hidden_dim,
        config: c,
        device: vs.device(),
    })
}

impl RNN for LSTM {
    type State = LSTMState;

    fn zero_state(&self, batch_dim: i64) -> Result<LSTMState> {
        let directions = if self.config.bidirectional { 2 } else { 1 };
        let shape = [
            self.config.num_layers * directions,
            batch_dim,
            self.hidden_dim,
        ];
        let kind = self.flat_weights[0].kind();
        Ok(LSTMState((
            Tensor::f_zeros(shape, (kind, self.device))?,
            Tensor::f_zeros(shape, (kind, self.device))?,
        )))
    }

    fn seq_init(&self, input: &Tensor, state: &LSTMState) -> Result<(Tensor, LSTMState)> {
        let LSTMState((h, c)) = state;
        let weights = self.flat_weights.iter().collect::<Vec<_>>();
        let (output, h, c) = input.f_lstm(
            &[h, c],
            &weights,
            self.config.has_biases,
            self.config.num_layers,
            self.config.dropout,
            self.config.train,
            self.config.bidirectional,
            self.config.batch_first,
        )?;
        Ok((output, LSTMState((h, c))))
    }
}
