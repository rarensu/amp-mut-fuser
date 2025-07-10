// This file orchestrates the container module.
// It declares submodules and re-exports public items.

pub mod core;
pub mod specialized;

pub use self::core::Container;

#[cfg(test)]
mod tests;
