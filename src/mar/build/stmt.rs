use aster::AstBuilder;
use mar::build::Builder;
use mar::repr::*;
use syntax::ast::{self, StmtKind};
use syntax::codemap::Span;
use syntax::ptr::P;

impl<'a, 'b: 'a> Builder<'a, 'b> {
    pub fn stmts(&mut self,
                 extent: CodeExtent,
                 mut block: BasicBlock,
                 stmts: &[ast::Stmt]) -> BasicBlock {
        for stmt in stmts {
            block = self.stmt(extent, block, stmt);
        }

        block
    }

    pub fn stmt(&mut self,
                extent: CodeExtent,
                block: BasicBlock,
                stmt: &ast::Stmt) -> BasicBlock {
        match stmt.node {
            StmtKind::Expr(ref expr) | StmtKind::Semi(ref expr) => {
                // Ignore empty statements.
                if expr_is_empty(expr) {
                    block
                } else {
                    self.expr(extent, block, expr)
                }
            }
            StmtKind::Local(ref local) => {
                self.local(extent, block, stmt.span, local)
            }
            StmtKind::Item(..) => {
                self.cx.span_bug(stmt.span, "Cannot handle item declarations yet");
            }
            StmtKind::Mac(ref mac) => {
                let (ref mac, _, _) = **mac;
                match self.mac(block, mac) {
                    Some(block) => block,
                    None => self.into(extent, block, stmt.clone()),
                }
            }
        }
    }

    fn local(&mut self,
             extent: CodeExtent,
             block: BasicBlock,
             span: Span,
             local: &P<ast::Local>) -> BasicBlock {
        if local.init.is_none() {
            self.cx.span_bug(span, &format!("Local variables need initializers at the moment"));
        }

        let block2 = self.expr(extent, block, &local.init.clone().unwrap());

        let init_stmt = if block == block2 {
            self.cfg.basic_blocks[block.index()].statements.pop().unwrap()
        } else {
            let block_index = block2.index() as usize - 1;
            {
                let decls = self.cfg.basic_blocks[block_index].decls().to_owned();

                let let_block = &mut self.cfg.basic_blocks[block2.index()];
                for (decl, ident) in decls {
                    if !let_block.decls.iter().any(|&(_, x)| x.name == ident.name) {
                        let_block.decls.push((decl, ident));
                    }
                }
            }
            let init_index = self.cfg.basic_blocks[block_index].statements.iter().enumerate()
                .filter(|&(_, block_statement)| {
                    match block_statement {
                        &Statement::Expr(..) => true,
                        _ => false,
                    }
                })
                .map(|(idx, _)| idx).next().unwrap();

            self.cfg.basic_blocks[block_index].statements.remove(init_index)
        };

        let init_stmt = Some(match init_stmt {
            Statement::Expr(ref stmt) => {
                match stmt.node {
                    ast::StmtKind::Semi(ref expr) | ast::StmtKind::Expr(ref expr) => expr,
                    _ => unreachable!(),
                }
            }
            _ => {
                panic!("something unexpected");
            }
        }.clone());

        let block = block2;

        for (decl, _) in self.get_decls_from_pat(&local.pat) {
            let lvalue = self.cfg.var_decl_data(decl).ident;

            let alias = self.find_decl(lvalue).map(|alias| {
                self.alias(block, span, alias)
            });

            self.schedule_drop(span, extent, decl, alias);
        }

        self.cfg.push(block, Statement::Let {
            span: span,
            pat: local.pat.clone(),
            ty: local.ty.clone(),
            // init: local.init.clone(),
            init: init_stmt,
        });

        block
    }

    fn alias(&mut self,
             block: BasicBlock,
             span: Span,
             decl: VarDecl) -> Alias {
        let lvalue = self.cfg.var_decl_data(decl).ident;

        let ast_builder = AstBuilder::new().span(span);
        let alias = ast_builder.id(format!("{}_shadowed_{}", lvalue, decl.index()));

        self.cfg.push(block, Statement::Let {
            span: span,
            pat: ast_builder.pat().id(alias),
            ty: None,
            init: Some(ast_builder.expr().id(lvalue)),
        });

        Alias {
            lvalue: alias,
            decl: decl,
        }
    }

    pub fn into_stmt(&mut self,
                     _extent: CodeExtent,
                     block: BasicBlock,
                     stmt: ast::Stmt) -> BasicBlock {
        self.cfg.push(block, Statement::Expr(stmt));
        block
    }
}

fn stmt_is_empty(stmt: &ast::Stmt) -> bool {
    match stmt.node {
        ast::StmtKind::Expr(ref e) | ast::StmtKind::Semi(ref e) => expr_is_empty(e),
        _ => false
    }
}

fn expr_is_empty(expr: &ast::Expr) -> bool {
    match expr.node {
        ast::ExprKind::Block(ref block) => {
            for stmt in block.stmts.iter() {
                if !stmt_is_empty(stmt) {
                    return false;
                }
            }

            true
        }
        _ => {
            false
        }
    }
}
