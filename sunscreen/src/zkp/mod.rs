use petgraph::Graph;
use sunscreen_runtime::CallSignature;
use sunscreen_zkp_backend::{BigInt, CompiledZkpProgram, Gadget, Operation as JitOperation};

use crate::Result;

use std::hash::Hash;
use std::sync::Arc;
use std::vec;
use std::{any::Any, cell::RefCell};

/**
 * An internal representation of a ZKP program specification.
 */
pub trait ZkpProgramFn {
    /**
     * Create a circuit from this specification.
     */
    fn build(&self) -> Result<ZkpFrontendCompilation>;

    /**
     * Gets the call signature for this program.
     */
    fn signature(&self) -> CallSignature;

    /**
     * Gets the name of this program.
     */
    fn name(&self) -> &str;
}

use std::fmt::Debug;

use petgraph::stable_graph::NodeIndex;
use sunscreen_compiler_common::{
    CompilationResult, Context, EdgeInfo, NodeInfo, Operation as OperationTrait, Render,
};

#[derive(Clone)]
pub enum Operation {
    PrivateInput(usize),
    PublicInput(usize),
    ConstantInput(usize),
    HiddenInput(usize),
    Constraint(BigInt),
    Constant(BigInt),
    InvokeGadget(Arc<dyn Gadget>),
    Add,
    Sub,
    Mul,
    Neg,
}

impl Hash for Operation {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        match self {
            Self::PrivateInput(x) => {
                state.write_u8(0);
                state.write_usize(*x);
            }
            Self::PublicInput(x) => {
                state.write_u8(1);
                state.write_usize(*x);
            }
            Self::HiddenInput(x) => {
                state.write_u8(2);
                state.write_usize(*x);
            }
            Self::Constraint(x) => {
                state.write_u8(3);
                x.hash(state);
            }
            Self::Constant(x) => {
                state.write_u8(4);
                x.hash(state);
            }
            Self::InvokeGadget(g) => {
                state.write_u8(5);
                g.type_id().hash(state);
            }
            Self::Add => state.write_u8(6),
            Self::Sub => state.write_u8(7),
            Self::Mul => state.write_u8(8),
            Self::Neg => state.write_u8(9),
            Self::ConstantInput(x) => {
                state.write_u8(10);
                x.hash(state);
            }
        }
    }
}

impl PartialEq for Operation {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::PrivateInput(x), Self::PrivateInput(y)) => x == y,
            (Self::PublicInput(x), Self::PublicInput(y)) => x == y,
            (Self::HiddenInput(x), Self::HiddenInput(y)) => x == y,
            (Self::Constraint(x), Self::Constraint(y)) => x == y,
            (Self::Constant(x), Self::Constant(y)) => x == y,
            (Self::InvokeGadget(x), Self::InvokeGadget(y)) => x.type_id() == y.type_id(),
            (Self::Add, Self::Add) => true,
            (Self::Sub, Self::Sub) => true,
            (Self::Mul, Self::Mul) => true,
            (Self::Neg, Self::Neg) => true,
            _ => false,
        }
    }
}

impl Eq for Operation {}

impl Debug for Operation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PrivateInput(x) => write!(f, "PrivateInput({x})"),
            Self::PublicInput(x) => write!(f, "PublicInput({x})"),
            Self::ConstantInput(x) => write!(f, "ConstantInput({x})"),
            Self::HiddenInput(x) => write!(f, "HiddenInput({x})"),
            Self::Constraint(x) => write!(f, "Constraint({x:#?})"),
            Self::Constant(x) => write!(f, "Constant({x:#?})"),
            Self::InvokeGadget(g) => write!(f, "InvokeGadget({})", g.debug_name()),
            Self::Add => write!(f, "Add"),
            Self::Sub => write!(f, "Sub"),
            Self::Mul => write!(f, "Mul"),
            Self::Neg => write!(f, "Neg"),
        }
    }
}

impl OperationTrait for Operation {
    fn is_binary(&self) -> bool {
        matches!(self, Operation::Add | Operation::Mul | Operation::Sub)
    }

    fn is_commutative(&self) -> bool {
        matches!(self, Operation::Add | Operation::Mul)
    }

    fn is_unary(&self) -> bool {
        matches!(self, Operation::Neg)
    }

    fn is_unordered(&self) -> bool {
        matches!(self, Operation::Constant(_))
    }

    fn is_ordered(&self) -> bool {
        matches!(self, Operation::InvokeGadget(_))
    }
}

impl Operation {
    pub fn is_add(&self) -> bool {
        matches!(self, Operation::Add)
    }

    pub fn is_sub(&self) -> bool {
        matches!(self, Operation::Sub)
    }

    pub fn is_mul(&self) -> bool {
        matches!(self, Operation::Mul)
    }

    pub fn is_neg(&self) -> bool {
        matches!(self, Operation::Neg)
    }

    pub fn is_private_input(&self) -> bool {
        matches!(self, Operation::PrivateInput(_))
    }

    pub fn is_public_input(&self) -> bool {
        matches!(self, Operation::PublicInput(_))
    }

    pub fn is_hidden_input(&self) -> bool {
        matches!(self, Operation::HiddenInput(_))
    }
}

/**
 * An implementation detail of a ZKP program. During compilation, it
 * tracks how many public and private inputs have been added.
 */
pub struct ZkpData {
    next_public_input: usize,
    next_private_input: usize,
    next_constant_input: usize,
}

impl ZkpData {
    /**
     * Creates a [`ZkpData`].
     */
    pub fn new() -> Self {
        Self {
            next_private_input: 0,
            next_public_input: 0,
            next_constant_input: 0,
        }
    }
}

impl Default for ZkpData {
    fn default() -> Self {
        Self::new()
    }
}

/**
 * An implementation detail of a ZKP program. During compilation, it holds
 * the graph of the program currently being constructed in an
 * [`#[zkp_program]`](crate::zkp_program) function.
 *
 * # Remarks
 * For internal use only.
 */
pub type ZkpContext = Context<Operation, ZkpData>;
/**
 * Contains the results of compiling a [`#[zkp_program]`](crate::zkp_program) function.
 *
 * # Remarks
 * For internal use only.
 */
pub type ZkpFrontendCompilation = CompilationResult<Operation>;

pub trait ZkpContextOps {
    fn add_public_input(&mut self) -> NodeIndex;

    fn add_private_input(&mut self) -> NodeIndex;

    fn add_constant_input(&mut self) -> NodeIndex;

    fn add_hidden_input(&mut self, gadget_arg_id: usize) -> NodeIndex;

    fn add_addition(&mut self, left: NodeIndex, right: NodeIndex) -> NodeIndex;

    fn add_multiplication(&mut self, left: NodeIndex, right: NodeIndex) -> NodeIndex;

    fn add_negate(&mut self, left: NodeIndex) -> NodeIndex;

    fn add_subtraction(&mut self, left: NodeIndex, right: NodeIndex) -> NodeIndex;

    fn add_constraint(&mut self, left: NodeIndex, val: &BigInt) -> NodeIndex;

    fn add_constant(&mut self, val: &BigInt) -> NodeIndex;

    fn add_invoke_gadget<G: Gadget>(&mut self, gadget: &Arc<G>) -> NodeIndex;
}

impl ZkpContextOps for ZkpContext {
    fn add_public_input(&mut self) -> NodeIndex {
        let node = self.add_node(Operation::PublicInput(self.data.next_public_input));
        self.data.next_public_input += 1;

        node
    }

    fn add_private_input(&mut self) -> NodeIndex {
        let node = self.add_node(Operation::PrivateInput(self.data.next_private_input));
        self.data.next_private_input += 1;

        node
    }

    fn add_constant_input(&mut self) -> NodeIndex {
        let node = self.add_node(Operation::ConstantInput(self.data.next_constant_input));
        self.data.next_constant_input += 1;

        node
    }

    fn add_hidden_input(&mut self, gadget_arg_id: usize) -> NodeIndex {
        self.add_node(Operation::HiddenInput(gadget_arg_id))
    }

    fn add_addition(&mut self, left: NodeIndex, right: NodeIndex) -> NodeIndex {
        self.add_binary_operation(Operation::Add, left, right)
    }

    fn add_multiplication(&mut self, left: NodeIndex, right: NodeIndex) -> NodeIndex {
        self.add_binary_operation(Operation::Mul, left, right)
    }

    fn add_negate(&mut self, left: NodeIndex) -> NodeIndex {
        self.add_unary_operation(Operation::Neg, left)
    }

    fn add_subtraction(&mut self, left: NodeIndex, right: NodeIndex) -> NodeIndex {
        self.add_binary_operation(Operation::Sub, left, right)
    }

    fn add_constraint(&mut self, left: NodeIndex, val: &BigInt) -> NodeIndex {
        let constraint = self.add_node(Operation::Constraint(*val));

        self.add_edge(left, constraint, EdgeInfo::Unordered);

        constraint
    }

    fn add_constant(&mut self, val: &BigInt) -> NodeIndex {
        self.add_node(Operation::Constant(*val))
    }

    fn add_invoke_gadget<G: Gadget>(&mut self, gadget: &Arc<G>) -> NodeIndex {
        self.add_node(Operation::InvokeGadget(gadget.clone()))
    }
}

impl Render for Operation {
    fn render(&self) -> String {
        format!("{:?}", self)
    }
}

thread_local! {
    /**
     * Contains the graph of a ZKP program during compilation. An
     * implementation detail and not for public consumption.
     */
    pub static CURRENT_ZKP_CTX: RefCell<Option<&'static mut ZkpContext>> = RefCell::new(None);
}

/**
 * Runs the specified closure, injecting the current
 * [`fhe_program`](crate::fhe_program) context.
 */
pub fn with_zkp_ctx<F, R>(f: F) -> R
where
    F: FnOnce(&mut ZkpContext) -> R,
{
    CURRENT_ZKP_CTX.with(|ctx| {
        let mut option = ctx.borrow_mut();
        let ctx = option
            .as_mut()
            .expect("Called with_zkp_ctx() outside of a context.");

        f(ctx)
    })
}

/**
 * Takes the parsed frontend program and turns into a format ready to be
 * run.
 */
pub(crate) fn compile(program: &ZkpFrontendCompilation) -> CompiledZkpProgram {
    let jit = program.0.map(
        |_, n| {
            let operation = match n.operation {
                Operation::PrivateInput(x) => JitOperation::PrivateInput(x),
                Operation::PublicInput(x) => JitOperation::PublicInput(x),
                Operation::ConstantInput(x) => JitOperation::ConstantInput(x),
                Operation::HiddenInput(x) => JitOperation::HiddenInput(x),
                Operation::InvokeGadget(ref g) => JitOperation::InvokeGadget(g.clone()),
                Operation::Add => JitOperation::Add,
                Operation::Mul => JitOperation::Mul,
                Operation::Neg => JitOperation::Neg,
                Operation::Sub => JitOperation::Sub,
                Operation::Constraint(x) => JitOperation::Constraint(x),
                Operation::Constant(x) => JitOperation::Constant(x),
            };

            NodeInfo { operation }
        },
        |_, e| *e,
    );

    // Convert in and out of Graph to compact all the node indices.
    let jit = Graph::from(jit).into();

    CompilationResult(jit)
}

/**
 * Invokes a gadget and adds its sub-circuit to the graph.
 *
 * # Panics
 * * Calling this function inside a [`with_zkp_ctx`] callback
 * * `gadget_inputs.len() != g.get_gadget_input_count()`
 */
pub fn invoke_gadget<G: Gadget>(g: G, gadget_inputs: &[NodeIndex]) -> Vec<NodeIndex> {
    let hidden_inputs_count = g.hidden_input_count();
    let gadget_input_count = g.gadget_input_count();

    assert_eq!(
        gadget_input_count,
        gadget_inputs.len(),
        "{} gadget input mismatch: Expected {gadget_input_count} arguments found {}",
        g.debug_name(),
        gadget_inputs.len()
    );

    let g = Arc::new(g);

    let mut hidden_inputs = vec![];

    with_zkp_ctx(|ctx| {
        let gadget = ctx.add_invoke_gadget(&g);

        for i in 0..hidden_inputs_count {
            let hidden_input = ctx.add_hidden_input(i);
            ctx.add_edge(gadget, hidden_input, EdgeInfo::Unary);

            hidden_inputs.push(hidden_input);
        }

        for (i, gadget_input) in gadget_inputs.iter().enumerate() {
            ctx.add_edge(*gadget_input, gadget, EdgeInfo::Ordered(i));
        }
    });

    g.gen_circuit(gadget_inputs, &hidden_inputs)
}