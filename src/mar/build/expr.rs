use aster::AstBuilder;
use mar::build::Builder;
use mar::build::scope::LoopScope;
use mar::repr::*;
use syntax::ast::{self, ExprKind};
use syntax::codemap::Span;
use syntax::ptr::P;

impl<'a, 'b: 'a> Builder<'a, 'b> {
    pub fn expr(&mut self,
                extent: CodeExtent,
                block: BasicBlock,
                expr: &P<ast::Expr>) -> BasicBlock {
        let expr = self.expand_moved(expr);

        // There's no reason for us to transform expressions if they don't contain any transitions.
        if !self.contains_transition(&expr) {
            return self.into(extent, block, expr);
        }

        match expr.node {
            ExprKind::Block(ref ast_block) => {
                self.into(extent, block, ast_block)
            }
            ExprKind::Continue(label) => {
                self.break_or_continue(expr.span,
                                       label.map(|label| label.node),
                                       block,
                                       |loop_scope| loop_scope.continue_block)
            }
            ExprKind::Break(label) => {
                self.break_or_continue(expr.span,
                                       label.map(|label| label.node),
                                       block,
                                       |loop_scope| loop_scope.break_block)
            }
            ExprKind::Ret(Some(_)) => {
                self.cx.span_fatal(expr.span, "cannot return a value");
            }
            ExprKind::Ret(None) => {
                self.exit_scope(expr.span, extent, block, END_BLOCK);
                self.start_new_block(expr.span, Some("AfterReturn"))
            }
            ExprKind::If(ref cond_expr, ref then_expr, ref else_expr) => {
                // FIXME: This does not handle the `cond_expr` containing a transition yet.

                let mut then_block = self.start_new_block(expr.span, Some("Then"));
                let mut else_block = self.start_new_block(expr.span, Some("Else"));

                self.terminate(expr.span, block, TerminatorKind::If {
                    cond: cond_expr.clone(),
                    targets: (then_block, else_block),
                });

                then_block = self.into(extent, then_block, then_expr);
                else_block = self.into(extent, else_block, else_expr);

                let join_block = self.start_new_block(expr.span, Some("IfJoin"));

                self.terminate(
                    then_expr.span,
                    then_block,
                    TerminatorKind::Goto { target: join_block });

                self.terminate(
                    match *else_expr {
                        Some(ref expr) => expr.span,
                        None => expr.span,
                    },
                    else_block,
                    TerminatorKind::Goto { target: join_block });

                join_block
            }
            ExprKind::Match(ref discriminant, ref arms) => {
                self.match_expr(extent, expr.span, block, discriminant.clone(), &arms)

            }
            ExprKind::Loop(ref body, label) => {
                self.expr_loop(extent, block, None, body, label)
            }
            ExprKind::While(ref cond_expr, ref body, label) => {
                self.expr_loop(extent, block, Some(cond_expr), body, label)
            }
            ExprKind::ForLoop(ref pat, ref expr, ref loop_block, label) => {
                // Desugar a for loop into:
                //
                // {
                //     let mut iter = ::std::iter::IntoIterator::into_iter($expr);
                //     'label: loop {
                //         match iter.next() {
                //             ::std::option::Option::Some($pat) => $loop_block,
                //             ::std::option::Option::None => break,
                //         }
                //     }
                // }
                let builder = AstBuilder::new().span(expr.span);

                // ::std::iter::IntoIterator::into_iter($expr)
                let into_iter = builder.expr().call()
                    .path()
                        .global()
                        .ids(&["std", "iter", "IntoIterator", "into_iter"])
                        .build()
                    .with_arg(expr.clone())
                    .build();

                // iter.next()
                let iter_next = builder.expr().method_call("next")
                    .id("__stateful_iter")
                    .build();

                // ::std::option::Option::Some($pat)
                let some_pat = builder.pat().enum_()
                    .global().ids(&["std", "option", "Option", "Some"]).build()
                    .pat().build(pat.clone())
                    .build();

                // $some_pat => $loop_block
                let some_arm = builder.arm()
                    .with_pat(some_pat)
                    .body().build_block(loop_block.clone());

                // ::std::option::Option::None
                let none_pat = builder.pat().path()
                    .global().ids(&["std", "option", "Option", "None"]).build();

                // $none_pat => break,
                let none_arm = builder.arm()
                    .with_pat(none_pat)
                    .body().break_();

                // match $iter_next() {
                //     Some($pat) => $block,
                //     None => break,
                // }
                let match_expr = builder.expr()
                    .match_().build(iter_next)
                    .with_arm(some_arm)
                    .with_arm(none_arm)
                    .build();

                // `loop { $match_expr; };`
                let mut loop_builder = builder.expr().loop_();

                if let Some(label) = label {
                    loop_builder = loop_builder.label(label.node);
                }

                let loop_ = loop_builder.block()
                    .stmt().build_expr(match_expr)
                    .build();

                // `let mut iter = $into_iter;`
                let iter = builder.stmt()
                    .let_().mut_id("__stateful_iter")
                    .build_expr(into_iter);

                // {
                //     $into_iter;
                //     $loop;
                // }
                let expr = builder.expr().block()
                    .with_stmt(iter)
                    .stmt().build_expr(loop_)
                    .build();

                self.expr(extent, block, &expr)
            }
            ExprKind::IfLet(ref pat, ref expr, ref then_block, ref else_block) => {
                // Desugar an if-let:
                //
                // match $expr {
                //     $pat => $then_block,
                //     _ => $else_block,
                // }
                let builder = AstBuilder::new().span(expr.span);

                // $then_pat => $then_block
                let then_arm = builder.arm()
                    .with_pat(pat.clone())
                    .body().build_block(then_block.clone());

                // _ => $else_block
                let else_arm = match *else_block {
                    Some(ref else_block) => builder.arm().pat().wild().body().build(else_block.clone()),
                    None => builder.arm().pat().wild().body().unit(),
                };

                // match $iter_next() {
                //     $pat => $then_block,
                //     _ => #else_block,
                // }
                let expr = builder.expr()
                    .match_().build(expr.clone())
                    .with_arm(then_arm)
                    .with_arm(else_arm)
                    .build();

                self.expr(extent, block, &expr)
            }
            ExprKind::WhileLet(ref pat, ref expr, ref then_block, label) => {
                // Desugar an while-let:
                //
                // 'label: loop {
                //     match $expr {
                //         $pat => $body_block,
                //         _ => break,
                //     }
                // }
                let builder = AstBuilder::new().span(expr.span);

                // $pat => $then_block
                let then_arm = builder.arm()
                    .with_pat(pat.clone())
                    .body().build_block(then_block.clone());

                // _ => break
                let else_arm = builder.arm()
                    .pat().wild()
                    .body().break_();

                // match $expr {
                //     $then_arm,
                //     $else_arm,
                // }
                let match_expr = builder.expr()
                    .match_().build(expr.clone())
                    .with_arm(then_arm)
                    .with_arm(else_arm)
                    .build();

                // `'$label: loop { $match_expr; };`
                let mut loop_builder = builder.expr().loop_();

                if let Some(label) = label {
                    loop_builder = loop_builder.label(label.node);
                }

                let loop_expr = loop_builder.block()
                    .stmt().build_expr(match_expr)
                    .build();

                self.expr(extent, block, &loop_expr)
            }
            _ => {
                self.cx.span_bug(expr.span,
                                 &format!("don't know how to handle {:#?} yet", expr))
            }
        }
    }

    fn expr_loop(&mut self,
                 extent: CodeExtent,
                 block: BasicBlock,
                 condition: Option<&P<ast::Expr>>,
                 body: &P<ast::Block>,
                 label: Option<ast::SpannedIdent>) -> BasicBlock {
        // [block] --> [loop_block] ~~> [loop_block_end] -1-> [exit_block]
        //                  ^                  |
        //                  |                  0
        //                  |                  |
        //                  |                  v
        //           [body_block_end] <~~~ [body_block]
        //
        // If `opt_cond_expr` is `None`, then the graph is somewhat simplified:
        //
        // [block] --> [loop_block / body_block ] ~~> [body_block_end]    [exit_block]
        //                         ^                          |
        //                         |                          |
        //                         +--------------------------+

        let loop_block = self.start_new_block(body.span, Some("Loop"));
        let exit_block = self.start_new_block(body.span, Some("LoopExit"));

        // start the loop
        self.terminate(
            body.span,
            block,
            TerminatorKind::Goto { target: loop_block });

        self.in_loop_scope(extent, label, loop_block, exit_block, |this| {
            // conduct the test, if necessary
            let body_block;
            if let Some(cond_expr) = condition {
                // FIXME: This does not yet handle the expr having a transition.

                body_block = this.start_new_block(cond_expr.span, Some("LoopBody"));

                this.terminate(
                    cond_expr.span,
                    loop_block,
                    TerminatorKind::If {
                        cond: cond_expr.clone(),
                        targets: (body_block, exit_block),
                    });
            } else {
                body_block = loop_block;
            }

            // execute the body, branching back to the test
            let body_block_end = this.into(extent, body_block, body);
            this.terminate(
                body.span,
                body_block_end,
                TerminatorKind::Goto { target: loop_block });

            // final point is exit_block
            exit_block
        })
    }

    fn break_or_continue<F>(&mut self,
                            span: Span,
                            label: Option<ast::Ident>,
                            block: BasicBlock,
                            exit_selector: F)
                            -> BasicBlock
        where F: FnOnce(&LoopScope) -> BasicBlock
    {
        let loop_scope = self.find_loop_scope(span, label);
        let exit_block = exit_selector(&loop_scope);
        self.exit_scope(span, loop_scope.extent, block, exit_block);

        // Even though we've exited `block`, there could be code following the break/continue. To
        // keep rust happy, we'll create a new block that has an edge to `block`, even though
        // control will never actually flow into this block.
        self.start_new_block(span, Some("AfterBreakOrContinue"))
    }
}
