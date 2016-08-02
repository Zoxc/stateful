use aster::AstBuilder;
use mar::repr::*;
use syntax::ast::{self, FunctionRetTy};
use syntax::ext::base::ExtCtxt;
use syntax::fold;
use syntax::ptr::P;

pub fn translate(cx: &ExtCtxt, mar: &Mar) -> Option<P<ast::Item>> {
    let ast_builder = AstBuilder::new().span(mar.span);

    let item_builder = ast_builder.item().fn_(mar.ident)
        .with_args(mar.fn_decl.inputs.iter().cloned());

    let item_builder = item_builder
        .build_return(return_type(mar))
        .generics().with(mar.generics.clone()).build();

    let builder = Builder {
        cx: cx,
        ast_builder: ast_builder,
        mar: mar,
    };

    let start_state_expr = builder.state_expr(mar.span, START_BLOCK);
    let (state_enum, state_default, state_arms) =
        builder.state_enum_default_and_arms();

    let closure_type;
    let wrapper_impl;
    
    match mar.state_machine_kind {
        StateMachineKind::Generator => {
            closure_type = quote_ty!(cx, Option<T>);
            wrapper_impl = quote_item!(cx,
                impl<S, T, F> Iterator for Wrapper<S, F>
                    where S: Default,
                          F: Fn(S) -> (Option<T>, S)
                {
                    type Item = T;

                    fn next(&mut self) -> Option<Self::Item> {
                        let old_state = ::std::mem::replace(&mut self.state, S::default());
                        let (value, next_state) = (self.next)(old_state);
                        self.state = next_state;
                        value
                    }
                }
            ).unwrap();
        }
        StateMachineKind::Async => {
            closure_type = quote_ty!(cx, Poll<T>);
            wrapper_impl = quote_item!(cx,
                impl<S, T, F> Future for Wrapper<S, F>
                    where S: Default,
                          F: Fn(S) -> (Poll<T>, S)
                {
                    type Item = T;

                    fn poll(&mut self) -> Poll<Self::Item> {
                        let old_state = ::std::mem::replace(&mut self.state, S::default());
                        let (value, next_state) = (self.next)(old_state);
                        self.state = next_state;
                        value
                    }
                }
            ).unwrap();
        }
    };

    let block = quote_block!(cx, {
        struct Wrapper<S, F> {
            state: S,
            next: F,
        }

        impl<S, T, F> Wrapper<S, F>
            where F: Fn(S) -> ($closure_type, S),
        {
            fn new(initial_state: S, next: F) -> Self {
                Wrapper {
                    state: initial_state,
                    next: next,
                }
            }
        }

        $wrapper_impl
        $state_enum
        $state_default

        Box::new(Wrapper::new(
            $start_state_expr,
            |mut state| {
                loop {
                    match state {
                        $state_arms
                    }
                }
            }
        ))
    });

    let item = item_builder.build(block);

    // Syntax extensions are not allowed to have any node ids, so we need to remove them before we
    // return the item to the caller.
    let item = strip_node_ids(item);

    Some(item)
}

fn return_type(mar: &Mar) -> P<ast::Ty> {
    let (builder, ty) = match mar.fn_decl.output {
        FunctionRetTy::None(span) | FunctionRetTy::Default(span) => {
            let builder = AstBuilder::new().span(span);
            (builder, builder.ty().unit())
        }
        FunctionRetTy::Ty(ref ty) => {
            (AstBuilder::new().span(ty.span), ty.clone())
        }
    };

    let ty = match mar.state_machine_kind {
        StateMachineKind::Generator => {
            builder.ty().object_sum()
                .iterator().build(ty)
                .with_generics(mar.generics.clone())
                .build()
        }
        StateMachineKind::Async => {
            let path = builder.path()
                .segment("Future")
                    .binding("Item").build(ty)
                    .build()
                .build();

            builder.ty().object_sum()
                .build_path(path)
                .with_generics(mar.generics.clone())
                .build()
        }
    };

    builder.ty().box_().build(ty)
}

fn strip_node_ids(item: P<ast::Item>) -> P<ast::Item> {
    struct Stripper;

    impl fold::Folder for Stripper {
        fn new_id(&mut self, _old_id: ast::NodeId) -> ast::NodeId {
            ast::DUMMY_NODE_ID
        }

        fn fold_mac(&mut self, mac: ast::Mac) -> ast::Mac {
            fold::noop_fold_mac(mac, self)
        }
    }

    let mut items = fold::Folder::fold_item(&mut Stripper, item);
    assert_eq!(items.len(), 1);
    items.pop().unwrap()
}

pub struct Builder<'a, 'b: 'a> {
    cx: &'a ExtCtxt<'b>,
    ast_builder: AstBuilder,
    mar: &'a Mar,
}

mod block;
mod state;
mod stmt;
