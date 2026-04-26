pub mod compressed_string;
pub mod historical;
pub mod markdown;
pub mod page;
pub mod reqwest_ext;
pub mod settings;
pub mod types;
pub mod url;

mod archiver;
mod json_ld;

pub use archiver::{Archiver, DefaultArchiver, MockArchiver};
pub use json_ld::JsonLd;
pub use json_ld::parse as parse_jsonld;
