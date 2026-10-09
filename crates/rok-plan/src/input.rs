//! Layered command input: built-in defaults < `-f` file < `-j` inline JSON <
//! explicit flags.
//!
//! Every layer is a JSON object, merged key by key (nested objects merge
//! recursively; anything else is replaced), then deserialized into the
//! command's argument type. Errors carry the JSON pointer of the offending
//! field.

use std::fmt;
use std::io::Read;
use std::path::Path;

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::FORMAT_VERSION;

/// Why a payload couldn't be turned into a command's arguments.
#[derive(Debug, thiserror::Error)]
pub enum InputError {
    #[error("could not read {source_name}: {source}")]
    Read {
        source_name: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse {source_name} as {format}: {message}")]
    Parse {
        source_name: String,
        format: Format,
        message: String,
    },
    #[error("{source_name} must be an object, found {found}")]
    NotObject {
        source_name: String,
        found: &'static str,
    },
    #[error(
        "payload format version {found} is newer than this rok-gen supports ({FORMAT_VERSION}); \
         upgrade rok-gen"
    )]
    UnsupportedVersion { found: u64 },
    #[error("invalid payload at {pointer}: {message}")]
    Invalid { pointer: String, message: String },
}

/// The syntax a payload file is written in, chosen by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    /// JSON. Comments are allowed (JSONC), so `.json` and `.jsonc` share it.
    Json,
    Yaml,
    Toml,
}

impl Format {
    /// `.yaml`/`.yml` and `.toml` by extension; anything else is JSON(C).
    #[must_use]
    pub fn from_path(path: &Path) -> Self {
        match path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("yaml" | "yml") => Self::Yaml,
            Some("toml") => Self::Toml,
            _ => Self::Json,
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Json => "JSON",
            Self::Yaml => "YAML",
            Self::Toml => "TOML",
        })
    }
}

/// Parses one payload layer and requires it to be an object.
///
/// # Errors
///
/// [`InputError::Parse`] on a syntax error, [`InputError::NotObject`] if the
/// document isn't an object.
pub fn parse(text: &str, format: Format, source_name: &str) -> Result<Value, InputError> {
    let parsed: Result<Value, String> = match format {
        Format::Json => serde_json::from_reader(json_comments::StripComments::new(text.as_bytes()))
            .map_err(|err| err.to_string()),
        Format::Yaml => serde_norway::from_str(text).map_err(|err| err.to_string()),
        Format::Toml => toml::from_str(text).map_err(|err| err.to_string()),
    };
    let value = parsed.map_err(|message| InputError::Parse {
        source_name: source_name.to_string(),
        format,
        message,
    })?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(InputError::NotObject {
            source_name: source_name.to_string(),
            found: kind_of(&value),
        })
    }
}

/// Reads the `-f` layer: a file path relative to `cwd`, or `-` for stdin
/// (read as JSON).
///
/// # Errors
///
/// [`InputError::Read`] if the file or stdin can't be read, plus anything
/// [`parse`] returns.
pub fn read_file_layer(arg: &Path, cwd: &Path, stdin: &mut dyn Read) -> Result<Value, InputError> {
    if arg == Path::new("-") {
        let mut text = String::new();
        stdin
            .read_to_string(&mut text)
            .map_err(|source| InputError::Read {
                source_name: "stdin".to_string(),
                source,
            })?;
        return parse(&text, Format::Json, "stdin");
    }
    let path = cwd.join(arg);
    let source_name = arg.display().to_string();
    let text = std::fs::read_to_string(&path).map_err(|source| InputError::Read {
        source_name: source_name.clone(),
        source,
    })?;
    parse(&text, Format::from_path(&path), &source_name)
}

/// Deep-merges `overlay` into `base`: objects merge key by key, everything
/// else is replaced.
pub fn merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.get_mut(&key) {
                    Some(slot) => merge(slot, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (slot, value) => *slot = value,
    }
}

/// The input layers for one command, lowest precedence first (defaults
/// come from the argument type itself).
#[derive(Debug, Default)]
pub struct Layers {
    pub file: Option<Value>,
    pub inline: Option<Value>,
    /// Only the flags that were actually passed.
    pub flags: Option<Value>,
}

/// A command's arguments plus the merged payload they came from (what
/// `--print-payload` shows).
#[derive(Debug)]
pub struct Resolved<T> {
    pub args: T,
    pub payload: Value,
}

/// Merges `layers` over `T::default()` and deserializes the result.
///
/// A `version` key, if present, is checked against [`FORMAT_VERSION`] and
/// stripped before deserializing.
///
/// # Errors
///
/// [`InputError::UnsupportedVersion`] for a newer payload format, or
/// [`InputError::Invalid`] (with the field's JSON pointer and, for a
/// misspelled key, a "did you mean" hint) if the merged payload doesn't fit
/// `T`.
pub fn resolve<T>(layers: Layers) -> Result<Resolved<T>, InputError>
where
    T: DeserializeOwned + Serialize + Default,
{
    let mut payload = serde_json::to_value(T::default()).map_err(|err| InputError::Invalid {
        pointer: String::new(),
        message: err.to_string(),
    })?;
    for layer in [layers.file, layers.inline, layers.flags]
        .into_iter()
        .flatten()
    {
        merge(&mut payload, layer);
    }

    if let Some(object) = payload.as_object_mut()
        && let Some(version) = object.remove("version")
    {
        match version.as_u64() {
            Some(found) if found > u64::from(FORMAT_VERSION) => {
                return Err(InputError::UnsupportedVersion { found });
            }
            Some(_) => {}
            None => {
                return Err(InputError::Invalid {
                    pointer: "/version".to_string(),
                    message: "expected a positive integer".to_string(),
                });
            }
        }
    }

    let args = serde_path_to_error::deserialize(payload.clone()).map_err(|err| {
        let mut pointer = pointer_of(err.path());
        let message = err.inner().to_string();
        let message = match unknown_field_hint(&message) {
            Some((field, hint)) => {
                pointer.push('/');
                pointer.push_str(&escape_pointer_token(&field));
                format!("{message}{hint}")
            }
            None => message,
        };
        InputError::Invalid { pointer, message }
    })?;

    Ok(Resolved { args, payload })
}

fn pointer_of(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;

    let mut pointer = String::new();
    for segment in path {
        pointer.push('/');
        match segment {
            Segment::Seq { index } => pointer.push_str(&index.to_string()),
            Segment::Map { key } => pointer.push_str(&escape_pointer_token(key)),
            Segment::Enum { variant } => pointer.push_str(&escape_pointer_token(variant)),
            Segment::Unknown => pointer.push('?'),
        }
    }
    pointer
}

fn escape_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// For serde's "unknown field `x`, expected one of `a`, `b`" message: the
/// unknown field name, and a " (did you mean `a`?)" hint when one of the
/// expected names is close.
fn unknown_field_hint(message: &str) -> Option<(String, String)> {
    let rest = message.strip_prefix("unknown field `")?;
    let names: Vec<&str> = rest.split('`').step_by(2).collect();
    let (unknown, expected) = names.split_first()?;
    let best = expected
        .iter()
        .map(|candidate| (edit_distance(unknown, candidate), *candidate))
        .min()
        .filter(|(distance, _)| *distance <= 2);
    let hint = best.map_or_else(String::new, |(_, name)| {
        format!(" (did you mean `{name}`?)")
    });
    Some(((*unknown).to_string(), hint))
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let current = row[j + 1];
            row[j + 1] = if ca == *cb {
                previous
            } else {
                1 + previous.min(row[j]).min(current)
            };
            previous = current;
        }
    }
    row[b.len()]
}

const fn kind_of(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}
