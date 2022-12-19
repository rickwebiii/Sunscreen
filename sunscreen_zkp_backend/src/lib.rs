#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

//! This crate contains ZKP backends for use with the
//! Sunscreen compiler and runtime.

#[cfg(feature = "bulletproofs")]
/**
 * Types for working with Bulletproofs as the ZKP backend.
 */
pub mod bulletproofs;

mod error;
mod exec;
mod jit;

use std::{
    any::Any,
    ops::{Add, Deref, Mul, Neg, Sub},
};

pub use crypto_bigint::UInt;
use crypto_bigint::U512;
pub use error::*;
pub use exec::ExecutableZkpProgram;
pub use jit::{jit_prover, jit_verifier, CompiledZkpProgram, Operation};
use petgraph::stable_graph::NodeIndex;
use serde::{Deserialize, Serialize};

// Converting between U512 and backend numeric types requires an
// assumption about endianess. We require little endian for now unless
// there's demand for carefully writing endian-aware code.
#[cfg(not(target_endian = "little"))]
compile_error!("This crate currently requires a little endian target architecture.");

/**
 * In ZKP circuits, it's often simpler for the prover to provide additional
 * inputs and prove they meet some criteria than to directly compute some
 * quantity. However, *something* must compute these additional inputs. Rather
 * than delegate this responsibility to the prover's application, we use
 * [`Gadget`]s.
 *
 * `Gadget`s bear some resemblance to a function call in programming
 * languages. They take `N` input values and compute `M` output values. These
 * outputs get assigned to the additional inputs. In addition to computing
 * these values, the `Gadget` describes the circuit to prove the hidden inputs
 * satisfy some constraints.
 *
 * # Remarks
 * Gadget methods seem to accept a superfluous `&self` argument. This serves
 * to ensure the trait is object-safe. Although legal, implementors generally
 * won't have data.
 *
 * The [`Gadget::gadget_input_count`] method is not marked as `const` to
 * maintain object-safety, but implementors should ensure the values these
 * functions return is always the same for a given gadget type.
 *
 * # Example
 * Suppose we want to decompose a native field element `x` into 8-bit
 * unsigned binary. Directly computing this with e.g. Lagrange interpolation
 * is cost prohibitive because `x` lives in a very large field (e.g.
 * Bulletproofs Scalar values are O(2^255)).
 *
 * We instead ask the prover to simply provide the binary decomposition
 * and prove that it's correct. To do this, we create a gadget. Its
 * [`compute_inputs`](Gadget::compute_inputs) method directly computes the
 * decomposition with shifting and masking. Then, the
 * [`gen_circuit`](Gadget::gen_circuit) method defined a circuit that proves
 * the following:
 * * Each hidden input is a 0 or 1
 * * x == 2^7 * b_7 + 2^6 * b_6 ... 2^0 * b_0
 *
 * and outputs (b_0..b_7)
 */
pub trait Gadget: Any {
    /**
     * Create the subcircuit for this gadget.
     * * `gadget_inputs` are the node indices of the gadget inputs.
     * * `hidden_inputs` are the nodes of the gadget's hidden inputs.
     *
     * Returns the node indices of the gadget outputs.
     *
     * # Remarks
     * `gadget_inputs.len()` is guaranteed to equal
     * `self.get_gadget_input_count()`.
     *
     * `hidden_inputs.len()` is guaranteed to equal
     * `self.get_hidden_input_count()`
     */
    fn gen_circuit(
        &self,
        gadget_inputs: &[NodeIndex],
        hidden_inputs: &[NodeIndex],
    ) -> Vec<NodeIndex>;

    /**
     * Compute the values for each of the hidden inputs from the given
     * gadget inputs.
     *
     * * # Remarks
     * The number of returned hidden input values must equal
     * [`hidden_input_count`](Gadget::hidden_input_count).
     */
    fn compute_inputs(&self, gadget_inputs: &[BigInt]) -> Vec<BigInt>;

    /**
     * Returns the expected number of gadget inputs.
     */
    fn gadget_input_count(&self) -> usize;

    /**
     * Returns the expected number of hidden inputs.
     */
    fn hidden_input_count(&self) -> usize;

    /**
     * The gadget's name used to implement Operation's [`Debug`] trait.
     */
    fn debug_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

#[derive(Clone, Serialize, Deserialize)]
/**
 * An R1CS proof.
 */
pub enum Proof {
    #[cfg(feature = "bulletproofs")]
    /**
     * A Bulletproofs R1CS proof.
     */
    Bulletproofs(Box<bulletproofs::BulletproofsR1CSProof>),

    /**
     * A custom proof type provided by an external crate.
     */
    Custom {
        /**
         * THe name of the proof system.
         */
        name: String,
        /**
         * The proof data.
         */
        data: Vec<u8>,
    },
}

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
/**
 * A large integer representing a backend-agnostic
 * field element.
 */
pub struct BigInt(U512);

impl<T> std::convert::From<T> for BigInt
where
    T: Into<U512>,
{
    fn from(x: T) -> Self {
        Self(x.into())
    }
}

impl Deref for BigInt {
    type Target = U512;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl BigInt {
    /**
     * Create a [`BigInt`] from the given limbs.
     */
    pub const fn from_words(val: [u64; 8]) -> Self {
        Self(U512::from_words(val))
    }

    /**
     * Create a [`BigInt`] from the given u32.
     */
    pub const fn from_u32(val: u32) -> Self {
        Self(U512::from_u32(val))
    }

    /**
     * Create a [`BigInt`] from the given hex string.
     */
    pub fn from_be_hex(hex_str: &str) -> Self {
        Self(U512::from_be_hex(hex_str))
    }

    /**
     * The value 0.
     */
    pub const ZERO: Self = Self(U512::ZERO);

    /**
     * The value 1.
     */
    pub const ONE: Self = Self(U512::ONE);
}

/**
 * The methods needed for a type to serve as a proof
 * system in the Sunscreen ecosystem.
 */
pub trait ZkpBackend {
    /**
     * Create a proof for the given executable Sunscreen
     * program with the given inputs.
     */
    fn prove(&self, graph: &ExecutableZkpProgram, inputs: &[BigInt]) -> Result<Proof>;

    /**
     * Verify the given proof for the given executable
     * Sunscreen program.
     */
    fn verify(&self, graph: &ExecutableZkpProgram, proof: &Proof) -> Result<()>;

    /**
     * JIT the given frontend-compiled ZKP program
     * to an executable Sunscreen program for use by
     * a prover.
     *
     * # Remarks
     * Implementors should generally just call
     * [`jit_prover<U>`](jit_prover), passing the
     * appropriate backend field type for U.
     */
    fn jit_prover(
        &self,
        prog: &CompiledZkpProgram,
        constant_inputs: &[BigInt],
        public_inputs: &[BigInt],
        private_inputs: &[BigInt],
    ) -> Result<ExecutableZkpProgram>;

    /**
     * JIT the given backend-compiled ZKP program to an
     * executable Sunscreen program for use by a verifier.
     *
     * # Remarks
     * Implementors should generally just call
     * [`jit_verifier<U>`](jit_verifier), passing the
     * appropriate backend field type for U.
     */
    fn jit_verifier(
        &self,
        prog: &CompiledZkpProgram,
        constant_inputs: &[BigInt],
        public_inputs: &[BigInt],
    ) -> Result<ExecutableZkpProgram>;
}

/**
 * Indicates the given type is a field used used in a
 * ZKP backend. E.g. Bulletproofs uses Ristretto `Scalar`
 * values.
 */
pub trait BackendField:
    Add<Self, Output = Self>
    + Sub<Self, Output = Self>
    + Mul<Self, Output = Self>
    + Neg<Output = Self>
    + Clone
    + TryFrom<BigInt, Error = Error>
    + ZkpInto<BigInt>
{
}

/**
 * See [`std::convert::From`]. This trait exists to avoid limitations
 * with foreign trait rules.
 */
pub trait ZkpFrom<T> {
    /**
     * See [`std::convert::From::from`].
     */
    fn from(val: T) -> Self;
}

/**
 * See [`std::convert::Into`]. This trait exists to avoid limitations
 * with foreign trait rules.
 */
pub trait ZkpInto<T> {
    /**
     * See [`std::convert::Into::into`].
     */
    fn into(self) -> T;
}

impl<T, U> ZkpInto<T> for U
where
    T: ZkpFrom<U>,
{
    fn into(self) -> T {
        T::from(self)
    }
}