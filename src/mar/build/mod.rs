use mar::repr::*;
use syntax::ast::{self, ItemKind};
use syntax::codemap::Span;
use syntax::ext::base::ExtCtxt;
use syntax::fold;
use syntax::ptr::P;

#[derive(Debug)]
pub struct CFG {
    basic_blocks: Vec<BasicBlockData>,
    var_decls: Vec<VarDeclData>,
}

pub struct Builder<'a, 'b: 'a> {
    cx: &'a ExtCtxt<'b>,
    state_machine_kind: StateMachineKind,
    cfg: CFG,
    scopes: Vec<scope::Scope>,
    loop_scopes: Vec<scope::LoopScope>,
    extents: Vec<CodeExtentData>,
}

#[derive(Debug)]
pub struct Error;

///////////////////////////////////////////////////////////////////////////
// construct() -- the main entry point for building SMIR for a function

pub fn construct(cx: &ExtCtxt,
                 item: P<ast::Item>,
                 state_machine_kind: StateMachineKind) -> Result<Mar, Error> {
    let item = assign_node_ids(item);

    let (fn_decl, unsafety, constness, abi, generics, ast_block) = match item.node {
        ItemKind::Fn(ref fn_decl, unsafety, constness, abi, ref generics, ref block) => {
            (fn_decl, unsafety, constness, abi, generics, block)
        }
        _ => {
            cx.span_err(item.span, "`state_machine` may only be applied to functions");
            return Err(Error);
        }
    };

    let mut builder = Builder {
        cx: cx,
        state_machine_kind: state_machine_kind,
        cfg: CFG {
            basic_blocks: vec![],
            var_decls: vec![],
        },
        scopes: vec![],
        loop_scopes: vec![],
        extents: vec![],
    };

    let extent = builder.start_new_extent();

    assert_eq!(builder.start_new_block(item.span, Some("Start")), START_BLOCK);
    assert_eq!(builder.start_new_block(item.span, Some("End")), END_BLOCK);

    let mut block = START_BLOCK;

    builder.push_scope(extent, block);

    // Register the arguments as declarations.
    builder.add_decls_from_pats(
        extent,
        block,
        fn_decl.inputs.iter().map(|arg| &arg.pat));

    block = builder.ast_block(extent, block, ast_block);

    let live_decls = builder.find_live_decls();

    builder.pop_scope(extent, block);

    builder.terminate(item.span, block, TerminatorKind::Goto { target: END_BLOCK });
    builder.terminate(item.span, END_BLOCK, TerminatorKind::Return);

    // The drops seem redundant, we are always moving values.
    for bb in &mut builder.cfg.basic_blocks {
        bb.statements.retain(|stmt| {
            match *stmt {
                Statement::Drop { .. } => false,
                _ => true
            }
        });
    }

    Ok(Mar {
        state_machine_kind: builder.state_machine_kind,
        span: item.span,
        ident: item.ident,
        fn_decl: fn_decl.clone(),
        unsafety: unsafety,
        constness: constness,
        abi: abi,
        generics: generics.clone(),
        input_decls: live_decls,
        basic_blocks: builder.cfg.basic_blocks,
        var_decls: builder.cfg.var_decls,
        extents: builder.extents,
    })
}

fn assign_node_ids(item: P<ast::Item>) -> P<ast::Item> {
    struct Assigner {
        next_node_id: ast::NodeId,
    }

    impl fold::Folder for Assigner {
        fn new_id(&mut self, old_id: ast::NodeId) -> ast::NodeId {
            assert_eq!(old_id, ast::DUMMY_NODE_ID);
            let node_id = self.next_node_id;

            let next_node_id = match self.next_node_id.checked_add(1) {
                Some(next_node_id) => next_node_id,
                None => { panic!("ran out of node ids!") }
            };
            self.next_node_id = next_node_id;

            node_id
        }

        fn fold_mac(&mut self, mac: ast::Mac) -> ast::Mac {
            fold::noop_fold_mac(mac, self)
        }
    }

    let mut assigner = Assigner { next_node_id: 1 };
    let mut items = fold::Folder::fold_item(&mut assigner, item);
    assert_eq!(items.len(), 1);
    items.pop().unwrap()
}

impl<'a, 'b: 'a> Builder<'a, 'b> {
    pub fn start_new_block(&mut self, span: Span, name: Option<&'static str>) -> BasicBlock {
        let decls = self.find_live_decls();
        self.cfg.start_new_block(span, name, decls)
    }

    pub fn start_new_extent(&mut self) -> CodeExtent {
        let extent = CodeExtent::new(self.extents.len());
        self.extents.push(CodeExtentData::Misc(0));

        extent
    }

    pub fn is_inside_loop(&self) -> bool {
        !self.loop_scopes.is_empty()
    }
}

///////////////////////////////////////////////////////////////////////////
// Builder methods are broken up into modules, depending on what kind
// of thing is being translated.

mod block;
mod cfg;
mod expr;
mod into;
mod mac;
mod matches;
mod moved;
mod scope;
mod stmt;
mod transition;
