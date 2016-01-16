use aster::AstBuilder;
use syntax::ast;
use syntax::visit;

pub trait ContainsTransition {
    fn contains_transition(&self, inside_loop: bool) -> bool;
}

impl ContainsTransition for ast::Block {
    fn contains_transition(&self, inside_loop: bool) -> bool {
        let mut visitor = ContainsTransitionVisitor::new(inside_loop);

        visit::Visitor::visit_block(&mut visitor, self);
        visitor.contains_transition
    }
}

impl ContainsTransition for ast::Stmt {
    fn contains_transition(&self, inside_loop: bool) -> bool {
        let mut visitor = ContainsTransitionVisitor::new(inside_loop);

        visit::Visitor::visit_stmt(&mut visitor, self);
        visitor.contains_transition
    }
}

impl ContainsTransition for ast::Expr {
    fn contains_transition(&self, inside_loop: bool) -> bool {
        let mut visitor = ContainsTransitionVisitor::new(inside_loop);

        visit::Visitor::visit_expr(&mut visitor, self);
        visitor.contains_transition
    }
}

struct ContainsTransitionVisitor {
    inside_loop: bool,
    contains_transition: bool,
}

impl ContainsTransitionVisitor {
    fn new(inside_loop: bool) -> Self {
        ContainsTransitionVisitor {
            inside_loop: inside_loop,
            contains_transition: false,
        }
    }
}

impl<'a> visit::Visitor<'a> for ContainsTransitionVisitor {
    fn visit_stmt(&mut self, stmt: &ast::Stmt) {
        match stmt.node {
            ast::Stmt_::StmtMac(ref mac, _, _) if is_yield_path(&mac.node.path) => {
                self.contains_transition = true;
            }
            _ => {
                visit::walk_stmt(self, stmt)
            }
        }
    }

    fn visit_expr(&mut self, expr: &ast::Expr) {
        match expr.node {
            ast::Expr_::ExprRet(Some(_)) => {
                self.contains_transition = true;
            }
            ast::Expr_::ExprBreak(_) if self.inside_loop => {
                self.contains_transition = true;
            }
            ast::Expr_::ExprAgain(_) if self.inside_loop => {
                self.contains_transition = true;
            }
            ast::Expr_::ExprMac(ref mac) if is_transition_path(&mac.node.path) => {
                self.contains_transition = true;
            }
            ast::Expr_::ExprPath(None, ref path) if is_transition_path(path) => {
                self.contains_transition = true;
            }
            _ => {
                visit::walk_expr(self, expr)
            }
        }
    }

    fn visit_mac(&mut self, _mac: &ast::Mac) { }
}

pub fn is_transition_path(path: &ast::Path) -> bool {
    if is_yield_path(path) {
        true
    } else {
        false
    }
}

pub fn is_yield_path(path: &ast::Path) -> bool {
    let builder = AstBuilder::new();
    let yield_ = builder.path()
        .id("yield_")
        .build();

    !path.global && path.segments == yield_.segments
}
