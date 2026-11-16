foreign_macro_declare::modules!(alpha, beta);

pub fn total() -> u32 {
    alpha::value() + beta::value()
}
