use serde::{Deserialize, Serialize, Serializer};

pub const INFERRED_TYPE: &str = "<inferred>";

fn skip_if_none<T>(opt: &Option<T>) -> bool {
    opt.is_none()
}

fn skip_if_empty<T>(vec: &Vec<T>) -> bool {
    vec.is_empty()
}

fn skip_if_inferred(s: &str) -> bool {
    s == INFERRED_TYPE
}

fn skip_if_false(b: &bool) -> bool {
    !*b
}

fn serialize_bool_as_int<S>(value: &bool, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_u8(if *value { 1 } else { 0 })
}

fn skip_if_empty_body(body: &Option<FunctionBodyInfo>) -> bool {
    body.as_ref().map(|b| b.is_empty()).unwrap_or(true)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForgenOutput {
    pub crates: Vec<CrateMetadata>,
    pub files: Vec<FileTypeInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileTypeInfo {
    #[serde(rename = "path")]
    pub source_file: String,
    pub items: Vec<ItemInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CrateMetadata {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub features: Vec<String>,
    #[serde(
        rename = "local",
        serialize_with = "serialize_bool_as_int",
        skip_serializing_if = "skip_if_false"
    )]
    pub local: bool,
}

impl CrateMetadata {
    pub fn new(name: String, version: Option<String>, features: Vec<String>, local: bool) -> Self {
        Self {
            name,
            version,
            features,
            local,
        }
    }
}

/// Reference to an item defined elsewhere (for cross-file references)
#[derive(Debug, Serialize, Deserialize)]
pub struct ItemRef {
    /// Path to the item (e.g., "std::vec::Vec" or "crate::module::Type")
    pub path: String,
    pub id: String,
    /// File where this is defined (for local items)
    #[serde(skip_serializing_if = "skip_if_none")]
    pub defined_in: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ItemInfo {
    Function {
        name: String,
        id: String,
        #[serde(skip_serializing_if = "skip_if_empty")]
        params: Vec<ParamInfo>,
        #[serde(rename = "ret")]
        return_type: String,
        #[serde(skip_serializing_if = "skip_if_empty_body")]
        body: Option<FunctionBodyInfo>,
    },
    Struct {
        name: String,
        id: String,
        #[serde(skip_serializing_if = "skip_if_empty")]
        fields: Vec<FieldInfo>,
    },
    Enum {
        name: String,
        id: String,
        variants: Vec<VariantInfo>,
    },
    Trait {
        name: String,
        id: String,
        #[serde(skip_serializing_if = "skip_if_empty")]
        items: Vec<TraitItemInfo>,
    },
    TypeAlias {
        name: String,
        id: String,
        target: String,
    },
    Const {
        name: String,
        id: String,
        ty: String,
    },
    Static {
        name: String,
        id: String,
        ty: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ParamInfo {
    pub name: String,
    #[serde(skip_serializing_if = "skip_if_inferred")]
    pub ty: String,
    #[serde(rename = "ref", skip_serializing_if = "skip_if_none")]
    pub type_ref: Option<ItemRef>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FieldInfo {
    pub name: String,
    pub ty: String,
    #[serde(rename = "ref", skip_serializing_if = "skip_if_none")]
    pub type_ref: Option<ItemRef>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct VariantInfo {
    pub name: String,
    #[serde(skip_serializing_if = "skip_if_empty")]
    pub fields: Vec<FieldInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TraitItemInfo {
    Function {
        name: String,
        #[serde(skip_serializing_if = "skip_if_empty")]
        params: Vec<ParamInfo>,
        #[serde(rename = "ret")]
        return_type: String,
    },
    TypeAlias {
        name: String,
    },
    Const {
        name: String,
        ty: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionBodyInfo {
    #[serde(skip_serializing_if = "skip_if_empty")]
    pub locals: Vec<LocalVarInfo>,
    #[serde(skip_serializing_if = "skip_if_empty")]
    pub closures: Vec<ClosureInfo>,
}

impl FunctionBodyInfo {
    pub fn is_empty(&self) -> bool {
        self.locals.is_empty() && self.closures.is_empty()
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LocalVarInfo {
    #[serde(skip_serializing_if = "skip_if_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "skip_if_inferred")]
    pub ty: String,
    pub id: usize,
    #[serde(rename = "mut", serialize_with = "serialize_bool_as_int")]
    pub mutable: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClosureInfo {
    pub id: usize,
    #[serde(skip_serializing_if = "skip_if_empty")]
    pub params: Vec<ParamInfo>,
    #[serde(rename = "ret", skip_serializing_if = "skip_if_inferred")]
    pub return_type: String,
}
