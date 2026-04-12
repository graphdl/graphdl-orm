// crates/arest/src/generators/mod.rs
//
// Generators: compile FFP state (cells of named-tuple facts) to target languages.
//
// Each generator is a pure function (state: &Object) -> String that walks the
// metamodel cells (Noun, FactType, Role, Constraint, ...) and emits target
// source code. Generators are the "output side" of SYSTEM:x = <o, D'>, dual to
// the compile platform primitive that turns readings into D'.

pub mod solidity;
pub mod fpga;
