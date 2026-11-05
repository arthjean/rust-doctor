

#[cfg(test)]
mod tests;


#[cfg(not(test))]
pub mod production;


#[cfg(feature = "test-util")]
pub mod util;


#[cfg(all(test, feature = "strict"))]
mod strict;



#[cfg(any(test, feature = "strict"))]
pub mod loose;

pub fn feature() -> usize {
    2
}
