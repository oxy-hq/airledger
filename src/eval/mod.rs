//! The evaluator layer — pure functions over the schema model.
//!
//! These are the parts of the Dart code that are *not* parsing and
//! *not* I/O: predicate eval (show_when), derived-field computation,
//! cell encode/decode for the Sheets wire format, and template
//! interpolation. Used by the form, the save path, and the
//! templates flow.

pub mod codec;
pub mod derive;
pub mod show_when;
pub mod template;

pub use codec::{decode, encode};
pub use derive::{apply_derives, run_derive};
pub use show_when::is_visible_given;
pub use template::TemplateInterpolator;
