





use std::collections::HashMap;

pub fn routes() -> HashMap<String, Vec<Box<(String, Vec<(String, u32)>, fn(&str) -> u32)>>> {
    HashMap::new()
}

pub type Routes = HashMap<String, Vec<Box<(String, Vec<(String, u32)>, fn(&str) -> u32)>>>;

pub fn named_routes() -> Routes {
    HashMap::new()
}
