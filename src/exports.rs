use std::sync::Mutex;

use lazy_static::lazy_static;

lazy_static! {
    #[cfg(unix)]
    pub static ref CURRENT_CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);
}

#[derive(Debug)]
pub struct CommandSpec {
    pub argv: Vec<String>,
    pub redirect_in: Option<String>,
    pub redirect_out: Option<String>,
    pub redirect_out_append: Option<String>,
    pub redirect_err: Option<String>,
    pub redirect_err_append: Option<String>,
}