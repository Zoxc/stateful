#![feature(plugin)]
#![plugin(stateful)]
#![allow(dead_code)]
#![allow(non_shorthand_field_patterns)]
#![allow(unused_assignments)]
#![allow(unused_imports)]
#![allow(unused_mut)]
#![allow(unused_variables)]

use std::iter::Iterator;

fn foo() -> usize { 5 }

#[generator]
fn gen() -> Box<Iterator<Item=usize>> {
    if true {
        let x = foo();
        yield_!(x);
    }
    /*
    for item in 1..5 {
        yield_!(item);
    }
    */
}

fn main() {
    for value in gen() {
        println!("{}", value);
    }
}
