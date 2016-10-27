use aster::AstBuilder;
use mar::indexed_vec::{Idx, IndexVec};
use std::fmt;
use std::ops::{Index, IndexMut};
use std::u32;
use syntax::abi;
use syntax::ast;
use syntax::codemap::Span;
use syntax::ptr::P;

macro_rules! newtype_index {
    ($name:ident, $debug_name:expr) => (
        #[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(u32);

        impl Idx for $name {
            fn new(value: usize) -> Self {
                assert!(value < (u32::MAX) as usize);
                $name(value as u32)
            }
            fn index(self) -> usize {
                self.0 as usize
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
                write!(fmt, "{}{}", $debug_name, self.0)
            }
        }
    )
}

#[derive(Copy, Clone, Debug)]
pub enum StateMachineKind {
    Generator,
    Async,
}

/// Lowered representation of a single function.
#[derive(Debug)]
pub struct Mar {
    pub state_machine_kind: StateMachineKind,

    /// List of basic blocks. References to basic block use a newtyped index type `BasicBlock`
    /// that indexes into this vector.
    pub basic_blocks: IndexVec<BasicBlock, BasicBlockData>,

    /*
    /// List of visibility (lexical) scopes; these are referenced by statements
    /// and used (eventually) for debuginfo. Indexed by a `VisibilityScope`.
    pub visibility_scopes: IndexVec<VisibilityScope, VisibilityScopeData>,
    */

    pub span: Span,
    pub ident: ast::Ident,
    pub fn_decl: P<ast::FnDecl>,
    pub unsafety: ast::Unsafety,
    pub abi: abi::Abi,
    pub generics: ast::Generics,

    pub local_decls: IndexVec<Local, LocalDecl>,

    /// List of extents. References to extents use a newtyped index type `CodeExtent` that indexes
    /// into this vector.
    pub extents: IndexVec<CodeExtent, CodeExtentData>,
}

/// Where execution begins
pub const START_BLOCK: BasicBlock = BasicBlock(0);

impl Mar {
    #[inline]
    pub fn basic_blocks(&self) -> &IndexVec<BasicBlock, BasicBlockData> {
        &self.basic_blocks
    }

    #[inline]
    pub fn basic_blocks_mut(&mut self) -> &mut IndexVec<BasicBlock, BasicBlockData> {
        &mut self.basic_blocks
    }

    pub fn code_extent_data(&self, extent: CodeExtent) -> &CodeExtentData {
        &self.extents[extent]
    }

    pub fn local_decl_data(&self, local: Local) -> &LocalDecl {
        &self.local_decls[local]
    }
}

impl Index<BasicBlock> for Mar {
    type Output = BasicBlockData;

    fn index(&self, index: BasicBlock) -> &BasicBlockData {
        &self.basic_blocks()[index]
    }
}

impl IndexMut<BasicBlock> for Mar {
    fn index_mut(&mut self, index: BasicBlock) -> &mut BasicBlockData {
        &mut self.basic_blocks_mut()[index]
    }
}

///////////////////////////////////////////////////////////////////////////
// Variables and temps

newtype_index!(Local, "decl");

pub const RETURN_POINTER: Local = Local(0);

#[derive(Debug, PartialEq)]
pub struct LocalDecl {
    pub mutability: ast::Mutability,
    pub ident: ast::Ident,
    pub ty: Option<P<ast::Ty>>,
    pub shadowed_decl: Option<Local>,
    pub span: Span,
}

#[derive(Debug)]
pub struct DeclaredDecl {
    pub decl: Local,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LiveDecl {
    Forward(Local),
    Active(Local),
    Moved(Local),
}

///////////////////////////////////////////////////////////////////////////
// BasicBlock and Terminator

newtype_index!(BasicBlock, "bb");

#[derive(Debug)]
pub struct BasicBlockData {
    pub span: Span,
    pub name: Option<&'static str>,
    pub decls: Vec<LiveDecl>,
    pub statements: Vec<Statement>,
    pub terminator: Option<Terminator>,
}

impl BasicBlockData {
    pub fn new(span: Span,
               name: Option<&'static str>,
               decls: Vec<LiveDecl>,
               terminator: Option<Terminator>) -> Self {
        BasicBlockData {
            span: span,
            name: name,
            decls: decls,
            statements: vec![],
            terminator: terminator,
        }
    }

    pub fn name(&self) -> Option<&'static str> {
        self.name
    }

    pub fn decls(&self) -> &[LiveDecl] {
        &self.decls
    }

    pub fn statements(&self) -> &[Statement] {
        &self.statements
    }

    pub fn terminator(&self) -> &Terminator {
        self.terminator.as_ref().expect("invalid terminator state")
    }

    pub fn terminator_mut(&mut self) -> &mut Terminator {
        self.terminator.as_mut().expect("invalid terminator state")
    }
}

#[derive(Debug)]
pub struct Terminator {
    pub span: Span,
    pub kind: TerminatorKind,
}

#[derive(Debug)]
pub enum TerminatorKind {
    /// block should have one successor in the graph; we jump there
    Goto {
        target: BasicBlock,
        end_scope: bool,
    },

    /// jump to branch 0 if this lvalue evaluates to true
    If {
        cond: Operand,
        targets: (BasicBlock, BasicBlock),
    },

    /// lvalue evaluates to some enum; jump depending on the branch
    Match {
        discr: Operand,
        targets: Vec<Arm>,
    },

    /// Indicates a normal return. The ReturnPointer lvalue should
    /// have been filled in by now. This should only occur in the
    /// `END_BLOCK`.
    Return,

    Await {
        target: BasicBlock,
    },

    /// jump to target on next iteration.
    Suspend {
        // FIXME: We don't yet support resuming the coroutine with a value yet.
        // lvalue: Lvalue,
        rvalue: P<ast::Expr>,
        target: BasicBlock,
    },
}

impl Terminator {
    pub fn successors(&self) -> Vec<BasicBlock> {
        match self.kind {
            TerminatorKind::Goto { target, .. } => vec![target],
            TerminatorKind::Match { ref targets, .. } => {
                targets.iter().map(|arm| arm.block).collect()
            }
            TerminatorKind::If { targets: (then, else_), .. } => vec![then, else_],
            TerminatorKind::Return => vec![],
            TerminatorKind::Await { target } => vec![target],
            TerminatorKind::Suspend { target, .. } => vec![target],
        }
    }

    pub fn successors_mut(&mut self) -> Vec<&mut BasicBlock> {
        match self.kind {
            TerminatorKind::Goto { ref mut target, .. } => vec![target],
            TerminatorKind::Match { ref mut targets, .. } => {
                targets.iter_mut().map(|arm| &mut arm.block).collect()
            }
            TerminatorKind::If { targets: (ref mut then, ref mut else_), .. } => {
                vec![then, else_]
            }
            TerminatorKind::Return => vec![],
            TerminatorKind::Await { ref mut target } => vec![target],
            TerminatorKind::Suspend { ref mut target, .. } => vec![target],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Arm {
    pub pats: Vec<P<ast::Pat>>,
    pub guard: Option<P<ast::Expr>>,
    pub block: BasicBlock,
}

///////////////////////////////////////////////////////////////////////////

#[derive(Clone, Debug, PartialEq)]
pub enum Lvalue {
    Local(Local),
}

impl Lvalue {
    pub fn to_expr(&self, local_decls: &IndexVec<Local, LocalDecl>) -> P<ast::Expr> {
        match *self {
            Lvalue::Local(ref local) => {
                let local_decl = &local_decls[*local];
                AstBuilder::new().span(local_decl.span).expr().id(local_decl.ident)
            }
        }
    }
}

///////////////////////////////////////////////////////////////////////////
// Operands

/// These are values that can appear inside an rvalue (or an index
/// lvalue). They are intentionally limited to prevent rvalues from
/// being nested in one another.
#[derive(Clone, Debug, PartialEq)]
pub enum Operand {
    Consume(Lvalue),
    Constant(Constant),
}

impl Operand {
    pub fn to_expr(&self, local_decls: &IndexVec<Local, LocalDecl>) -> P<ast::Expr> {
        match *self {
            Operand::Consume(ref lvalue) => lvalue.to_expr(local_decls),
            Operand::Constant(ref constant) => constant.to_expr(),
        }
    }
}

///////////////////////////////////////////////////////////////////////////
/// Rvalues

#[derive(Clone, Debug, PartialEq)]
pub enum Rvalue {
    /// x (either a move or copy, depending on type of x)
    Use(Operand),

    /// Create an aggregate value, like a tuple or struct.  This is
    /// only needed because we want to distinguish `dest = Foo { x:
    /// ..., y: ... }` from `dest.x = ...; dest.y = ...;` in the case
    /// that `Foo` has a destructor. These rvalues can be optimized
    /// away after type-checking and before lowering.
    Aggregate(AggregateKind, Vec<Operand>),
}

impl Rvalue {
    pub fn to_expr(&self, local_decls: &IndexVec<Local, LocalDecl>) -> P<ast::Expr> {
        match *self {
            Rvalue::Use(ref lvalue) => lvalue.to_expr(local_decls),
            Rvalue::Aggregate(ref kind, ref items) => {
                match *kind {
                    AggregateKind::Tuple => {
                        AstBuilder::new().expr().tuple()
                            .with_exprs(items.iter().map(|item| item.to_expr(local_decls)))
                            .build()
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AggregateKind {
    Tuple,
}

///////////////////////////////////////////////////////////////////////////
/// Constants
///
/// Two constants are equal if they are the same constant. Note that
/// this does not necessarily mean that they are "==" in Rust -- in
/// particular one must be wary of `NaN`!

#[derive(Clone, Debug, PartialEq)]
pub struct Constant {
    pub span: Span,
    pub literal: P<ast::Lit>,
}


impl Constant {
    pub fn to_expr(&self) -> P<ast::Expr> {
        AstBuilder::new().span(self.span).expr()
            .build_lit(self.literal.clone())
    }
}

///////////////////////////////////////////////////////////////////////////
// Statements

#[derive(Debug)]
pub enum Statement {
    Expr(ast::Stmt),
    Declare(Local),
    Assign {
        span: Span,
        lvalue: Lvalue,
        rvalue: Rvalue,
    },
    Call {
        span: Span,
        fun: Operand,
        args: Vec<Operand>,
    },
    MethodCall {
        span: Span,
        ident: ast::SpannedIdent,
        tys: Vec<P<ast::Ty>>,
        args: Vec<Operand>,
    },
    Drop {
        lvalue: Local,
        moved: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct ShadowedDecl {
    pub lvalue: ast::Ident,
    pub decl: Local,
}

///////////////////////////////////////////////////////////////////////////
// Code Extents

newtype_index!(CodeExtent, "extent");

#[derive(Debug)]
pub enum CodeExtentData {
    Misc(ast::NodeId),

    ParameterScope { fn_id: ast::NodeId, body_id: ast::NodeId },

    // extent of code following a `let id = expr;` binding in a block
    Remainder(BlockRemainder),
}

/// Represents a subscope of `block` for a binding that is introduced
/// by `block.stmts[first_statement_index]`. Such subscopes represent
/// a suffix of the block. Note that each subscope does not include
/// the initializer expression, if any, for the statement indexed by
/// `first_statement_index`.
///
/// For example, given `{ let (a, b) = EXPR_1; let c = EXPR_2; ... }`:
///
/// * the subscope with `first_statement_index == 0` is scope of both
///   `a` and `b`; it does not include `EXPR_1`, but does include
///   everything after that first `let`. (If you want a scope that
///   includes `EXPR_1` as well, then do not use `CodeExtentData::Remainder`,
///   but instead another `CodeExtent` that encompasses the whole block,
///   e.g. `CodeExtentData::Misc`.
///
/// * the subscope with `first_statement_index == 1` is scope of `c`,
///   and thus does not include `EXPR_2`, but covers the `...`.
#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Hash, Debug, Copy)]
pub struct BlockRemainder {
    pub block: ast::NodeId,
    pub first_statement_index: u32,
}
