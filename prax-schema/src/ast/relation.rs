//! Relation analysis for the Prax schema AST.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::ReferentialAction;

/// The type of relation between two models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationType {
    /// One-to-one relation.
    OneToOne,
    /// One-to-many relation.
    OneToMany,
    /// Many-to-one relation (inverse of one-to-many).
    ManyToOne,
    /// Many-to-many relation.
    ManyToMany,
}

impl RelationType {
    /// Check if this is a "to-one" relation.
    pub fn is_to_one(&self) -> bool {
        matches!(self, Self::OneToOne | Self::ManyToOne)
    }

    /// Check if this is a "to-many" relation.
    pub fn is_to_many(&self) -> bool {
        matches!(self, Self::OneToMany | Self::ManyToMany)
    }

    /// Check if this is a "from-many" relation.
    pub fn is_from_many(&self) -> bool {
        matches!(self, Self::ManyToOne | Self::ManyToMany)
    }
}

impl std::fmt::Display for RelationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OneToOne => write!(f, "1:1"),
            Self::OneToMany => write!(f, "1:n"),
            Self::ManyToOne => write!(f, "n:1"),
            Self::ManyToMany => write!(f, "m:n"),
        }
    }
}

/// A resolved relation between two models.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Relation {
    /// Relation name (for disambiguation when multiple relations exist).
    pub name: Option<SmolStr>,
    /// The model containing the foreign key.
    pub from_model: SmolStr,
    /// The field on the from model.
    pub from_field: SmolStr,
    /// The foreign key field(s) on the from model.
    pub from_fields: Vec<SmolStr>,
    /// The model being referenced.
    pub to_model: SmolStr,
    /// The field on the to model (back-relation).
    pub to_field: Option<SmolStr>,
    /// The referenced field(s) on the to model.
    pub to_fields: Vec<SmolStr>,
    /// The type of relation.
    pub relation_type: RelationType,
    /// On delete action.
    pub on_delete: Option<ReferentialAction>,
    /// On update action.
    pub on_update: Option<ReferentialAction>,
    /// Custom foreign key constraint name in the database.
    pub map: Option<SmolStr>,
}

impl Relation {
    /// Create a new relation.
    pub fn new(
        from_model: impl Into<SmolStr>,
        from_field: impl Into<SmolStr>,
        to_model: impl Into<SmolStr>,
        relation_type: RelationType,
    ) -> Self {
        Self {
            name: None,
            from_model: from_model.into(),
            from_field: from_field.into(),
            from_fields: vec![],
            to_model: to_model.into(),
            to_field: None,
            to_fields: vec![],
            relation_type,
            on_delete: None,
            on_update: None,
            map: None,
        }
    }

    /// Set the relation name.
    pub fn with_name(mut self, name: impl Into<SmolStr>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the foreign key fields.
    pub fn with_from_fields(mut self, fields: Vec<SmolStr>) -> Self {
        self.from_fields = fields;
        self
    }

    /// Set the referenced fields.
    pub fn with_to_fields(mut self, fields: Vec<SmolStr>) -> Self {
        self.to_fields = fields;
        self
    }

    /// Set the back-relation field.
    pub fn with_to_field(mut self, field: impl Into<SmolStr>) -> Self {
        self.to_field = Some(field.into());
        self
    }

    /// Set the on delete action.
    pub fn with_on_delete(mut self, action: ReferentialAction) -> Self {
        self.on_delete = Some(action);
        self
    }

    /// Set the on update action.
    pub fn with_on_update(mut self, action: ReferentialAction) -> Self {
        self.on_update = Some(action);
        self
    }

    /// Set the custom foreign key constraint name.
    pub fn with_map(mut self, name: impl Into<SmolStr>) -> Self {
        self.map = Some(name.into());
        self
    }

    /// Check if this is an implicit many-to-many relation.
    pub fn is_implicit_many_to_many(&self) -> bool {
        self.relation_type == RelationType::ManyToMany && self.from_fields.is_empty()
    }

    /// Get the join table name for many-to-many relations.
    pub fn join_table_name(&self) -> Option<String> {
        if self.relation_type != RelationType::ManyToMany {
            return None;
        }

        // Sort model names for consistent naming
        let mut names = [self.from_model.as_str(), self.to_model.as_str()];
        names.sort();

        Some(format!("_{}_to_{}", names[0], names[1]))
    }
}

/// Index definition for a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Index {
    /// Index name (auto-generated if not specified).
    pub name: Option<SmolStr>,
    /// Fields included in the index.
    pub fields: Vec<IndexField>,
    /// Whether this is a unique index.
    pub is_unique: bool,
    /// Index type (btree, hash, etc.).
    pub index_type: Option<IndexType>,
    /// Vector distance operation (for HNSW/IVFFlat indexes).
    pub vector_ops: Option<VectorOps>,
    /// HNSW m parameter (max connections per layer, default 16).
    pub hnsw_m: Option<u32>,
    /// HNSW ef_construction parameter (size of candidate list during build, default 64).
    pub hnsw_ef_construction: Option<u32>,
    /// IVFFlat lists parameter (number of inverted lists, default 100).
    pub ivfflat_lists: Option<u32>,
}

impl Index {
    /// Create a new index.
    pub fn new(fields: Vec<IndexField>) -> Self {
        Self {
            name: None,
            fields,
            is_unique: false,
            index_type: None,
            vector_ops: None,
            hnsw_m: None,
            hnsw_ef_construction: None,
            ivfflat_lists: None,
        }
    }

    /// Create a unique index.
    pub fn unique(fields: Vec<IndexField>) -> Self {
        Self {
            name: None,
            fields,
            is_unique: true,
            index_type: None,
            vector_ops: None,
            hnsw_m: None,
            hnsw_ef_construction: None,
            ivfflat_lists: None,
        }
    }

    /// Set the index name.
    pub fn with_name(mut self, name: impl Into<SmolStr>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Set the index type.
    pub fn with_type(mut self, index_type: IndexType) -> Self {
        self.index_type = Some(index_type);
        self
    }

    /// Set the vector distance operation.
    pub fn with_vector_ops(mut self, ops: VectorOps) -> Self {
        self.vector_ops = Some(ops);
        self
    }

    /// Set HNSW m parameter.
    pub fn with_hnsw_m(mut self, m: u32) -> Self {
        self.hnsw_m = Some(m);
        self
    }

    /// Set HNSW ef_construction parameter.
    pub fn with_hnsw_ef_construction(mut self, ef: u32) -> Self {
        self.hnsw_ef_construction = Some(ef);
        self
    }

    /// Set IVFFlat lists parameter.
    pub fn with_ivfflat_lists(mut self, lists: u32) -> Self {
        self.ivfflat_lists = Some(lists);
        self
    }

    /// Check if this is a vector index.
    pub fn is_vector_index(&self) -> bool {
        self.index_type
            .as_ref()
            .is_some_and(|t| t.is_vector_index())
    }
}

/// A field in an index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexField {
    /// Field name.
    pub name: SmolStr,
    /// Sort order.
    pub sort: SortOrder,
}

impl IndexField {
    /// Create a new index field with ascending order.
    pub fn asc(name: impl Into<SmolStr>) -> Self {
        Self {
            name: name.into(),
            sort: SortOrder::Asc,
        }
    }

    /// Create a new index field with descending order.
    pub fn desc(name: impl Into<SmolStr>) -> Self {
        Self {
            name: name.into(),
            sort: SortOrder::Desc,
        }
    }
}

/// Sort order for index fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SortOrder {
    /// Ascending order.
    #[default]
    Asc,
    /// Descending order.
    Desc,
}

/// Index type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IndexType {
    /// B-tree index (default).
    BTree,
    /// Hash index.
    Hash,
    /// GiST index (PostgreSQL).
    Gist,
    /// GIN index (PostgreSQL).
    Gin,
    /// Full-text search index.
    FullText,
    /// BRIN index (PostgreSQL - Block Range Index).
    Brin,
    /// HNSW index for vector similarity search (pgvector).
    Hnsw,
    /// IVFFlat index for vector similarity search (pgvector).
    IvfFlat,
}

impl IndexType {
    /// Parse from string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "btree" => Some(Self::BTree),
            "hash" => Some(Self::Hash),
            "gist" => Some(Self::Gist),
            "gin" => Some(Self::Gin),
            "fulltext" => Some(Self::FullText),
            "brin" => Some(Self::Brin),
            "hnsw" => Some(Self::Hnsw),
            "ivfflat" => Some(Self::IvfFlat),
            _ => None,
        }
    }

    /// Check if this is a vector index type.
    pub fn is_vector_index(&self) -> bool {
        matches!(self, Self::Hnsw | Self::IvfFlat)
    }

    /// Get the SQL name for this index type.
    pub fn as_sql(&self) -> &'static str {
        match self {
            Self::BTree => "BTREE",
            Self::Hash => "HASH",
            Self::Gist => "GIST",
            Self::Gin => "GIN",
            Self::FullText => "GIN", // Full-text uses GIN in PostgreSQL
            Self::Brin => "BRIN",
            Self::Hnsw => "hnsw",
            Self::IvfFlat => "ivfflat",
        }
    }
}

/// Vector distance operation for similarity search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum VectorOps {
    /// Cosine distance (1 - cosine_similarity).
    #[default]
    Cosine,
    /// L2 (Euclidean) distance.
    L2,
    /// Inner product (negative dot product for max inner product search).
    InnerProduct,
}

impl VectorOps {
    /// Parse from string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "cosine" | "vector_cosine_ops" => Some(Self::Cosine),
            "l2" | "vector_l2_ops" | "euclidean" => Some(Self::L2),
            "ip" | "inner_product" | "vector_ip_ops" | "innerproduct" => Some(Self::InnerProduct),
            _ => None,
        }
    }

    /// Get the PostgreSQL operator class name for pgvector.
    pub fn as_ops_class(&self) -> &'static str {
        match self {
            Self::Cosine => "vector_cosine_ops",
            Self::L2 => "vector_l2_ops",
            Self::InnerProduct => "vector_ip_ops",
        }
    }

    /// Get the PostgreSQL distance operator.
    pub fn as_operator(&self) -> &'static str {
        match self {
            Self::Cosine => "<=>",
            Self::L2 => "<->",
            Self::InnerProduct => "<#>",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== RelationType Tests ====================

    #[test]
    fn test_relation_type_one_to_one() {
        let rt = RelationType::OneToOne;
        assert!(rt.is_to_one());
        assert!(!rt.is_to_many());
        assert!(!rt.is_from_many());
    }

    #[test]
    fn test_relation_type_one_to_many() {
        let rt = RelationType::OneToMany;
        assert!(!rt.is_to_one());
        assert!(rt.is_to_many());
        assert!(!rt.is_from_many());
    }

    #[test]
    fn test_relation_type_many_to_one() {
        let rt = RelationType::ManyToOne;
        assert!(rt.is_to_one());
        assert!(!rt.is_to_many());
        assert!(rt.is_from_many());
    }

    #[test]
    fn test_relation_type_many_to_many() {
        let rt = RelationType::ManyToMany;
        assert!(!rt.is_to_one());
        assert!(rt.is_to_many());
        assert!(rt.is_from_many());
    }

    #[test]
    fn test_relation_type_display() {
        assert_eq!(format!("{}", RelationType::OneToOne), "1:1");
        assert_eq!(format!("{}", RelationType::OneToMany), "1:n");
        assert_eq!(format!("{}", RelationType::ManyToOne), "n:1");
        assert_eq!(format!("{}", RelationType::ManyToMany), "m:n");
    }

    #[test]
    fn test_relation_type_equality() {
        assert_eq!(RelationType::OneToOne, RelationType::OneToOne);
        assert_ne!(RelationType::OneToOne, RelationType::OneToMany);
    }

    // ==================== Relation Tests ====================

    #[test]
    fn test_relation_new() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne);

        assert!(rel.name.is_none());
        assert_eq!(rel.from_model.as_str(), "Post");
        assert_eq!(rel.from_field.as_str(), "author");
        assert_eq!(rel.to_model.as_str(), "User");
        assert!(rel.to_field.is_none());
        assert_eq!(rel.relation_type, RelationType::ManyToOne);
        assert!(rel.on_delete.is_none());
        assert!(rel.on_update.is_none());
    }

    #[test]
    fn test_relation_with_name() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne)
            .with_name("PostAuthor");

        assert_eq!(rel.name, Some("PostAuthor".into()));
    }

    #[test]
    fn test_relation_with_from_fields() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne)
            .with_from_fields(vec!["author_id".into()]);

        assert_eq!(rel.from_fields, vec!["author_id".to_string()]);
    }

    #[test]
    fn test_relation_with_to_fields() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne)
            .with_to_fields(vec!["id".into()]);

        assert_eq!(rel.to_fields, vec!["id".to_string()]);
    }

    #[test]
    fn test_relation_with_to_field() {
        let rel =
            Relation::new("Post", "author", "User", RelationType::ManyToOne).with_to_field("posts");

        assert_eq!(rel.to_field, Some("posts".into()));
    }

    #[test]
    fn test_relation_with_on_delete() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne)
            .with_on_delete(ReferentialAction::Cascade);

        assert_eq!(rel.on_delete, Some(ReferentialAction::Cascade));
    }

    #[test]
    fn test_relation_with_on_update() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne)
            .with_on_update(ReferentialAction::Restrict);

        assert_eq!(rel.on_update, Some(ReferentialAction::Restrict));
    }

    #[test]
    fn test_relation_is_implicit_many_to_many_true() {
        let rel = Relation::new("Post", "tags", "Tag", RelationType::ManyToMany);
        assert!(rel.is_implicit_many_to_many());
    }

    #[test]
    fn test_relation_is_implicit_many_to_many_false_explicit() {
        let rel = Relation::new("Post", "tags", "Tag", RelationType::ManyToMany)
            .with_from_fields(vec!["post_id".into()]);
        assert!(!rel.is_implicit_many_to_many());
    }

    #[test]
    fn test_relation_is_implicit_many_to_many_false_not_mtm() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne);
        assert!(!rel.is_implicit_many_to_many());
    }

    #[test]
    fn test_relation_join_table_name_mtm() {
        let rel = Relation::new("Post", "tags", "Tag", RelationType::ManyToMany);
        assert_eq!(rel.join_table_name(), Some("_Post_to_Tag".to_string()));
    }

    #[test]
    fn test_relation_join_table_name_mtm_sorted() {
        // Should sort alphabetically
        let rel = Relation::new("Tag", "posts", "Post", RelationType::ManyToMany);
        assert_eq!(rel.join_table_name(), Some("_Post_to_Tag".to_string()));
    }

    #[test]
    fn test_relation_join_table_name_not_mtm() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne);
        assert!(rel.join_table_name().is_none());
    }

    #[test]
    fn test_relation_builder_chain() {
        let rel = Relation::new("Post", "author", "User", RelationType::ManyToOne)
            .with_name("PostAuthor")
            .with_from_fields(vec!["author_id".into()])
            .with_to_fields(vec!["id".into()])
            .with_to_field("posts")
            .with_on_delete(ReferentialAction::Cascade)
            .with_on_update(ReferentialAction::Restrict);

        assert_eq!(rel.name, Some("PostAuthor".into()));
        assert_eq!(rel.from_fields.len(), 1);
        assert_eq!(rel.to_fields.len(), 1);
        assert!(rel.to_field.is_some());
        assert!(rel.on_delete.is_some());
        assert!(rel.on_update.is_some());
    }

    #[test]
    fn test_relation_equality() {
        let rel1 = Relation::new("Post", "author", "User", RelationType::ManyToOne);
        let rel2 = Relation::new("Post", "author", "User", RelationType::ManyToOne);

        assert_eq!(rel1, rel2);
    }

    // ==================== Index Tests ====================

    #[test]
    fn test_index_new() {
        let idx = Index::new(vec![IndexField::asc("email")]);

        assert!(idx.name.is_none());
        assert_eq!(idx.fields.len(), 1);
        assert!(!idx.is_unique);
        assert!(idx.index_type.is_none());
    }

    #[test]
    fn test_index_unique() {
        let idx = Index::unique(vec![IndexField::asc("email")]);

        assert!(idx.is_unique);
    }

    #[test]
    fn test_index_with_name() {
        let idx = Index::new(vec![IndexField::asc("email")]).with_name("idx_user_email");

        assert_eq!(idx.name, Some("idx_user_email".into()));
    }

    #[test]
    fn test_index_with_type() {
        let idx = Index::new(vec![IndexField::asc("data")]).with_type(IndexType::Gin);

        assert_eq!(idx.index_type, Some(IndexType::Gin));
    }

    #[test]
    fn test_index_multiple_fields() {
        let idx = Index::unique(vec![
            IndexField::asc("first_name"),
            IndexField::asc("last_name"),
        ]);

        assert_eq!(idx.fields.len(), 2);
    }

    // ==================== IndexField Tests ====================

    #[test]
    fn test_index_field_asc() {
        let field = IndexField::asc("email");

        assert_eq!(field.name.as_str(), "email");
        assert_eq!(field.sort, SortOrder::Asc);
    }

    #[test]
    fn test_index_field_desc() {
        let field = IndexField::desc("created_at");

        assert_eq!(field.name.as_str(), "created_at");
        assert_eq!(field.sort, SortOrder::Desc);
    }

    #[test]
    fn test_index_field_equality() {
        let f1 = IndexField::asc("email");
        let f2 = IndexField::asc("email");
        let f3 = IndexField::desc("email");

        assert_eq!(f1, f2);
        assert_ne!(f1, f3);
    }

    // ==================== SortOrder Tests ====================

    #[test]
    fn test_sort_order_default() {
        let order = SortOrder::default();
        assert_eq!(order, SortOrder::Asc);
    }

    #[test]
    fn test_sort_order_equality() {
        assert_eq!(SortOrder::Asc, SortOrder::Asc);
        assert_eq!(SortOrder::Desc, SortOrder::Desc);
        assert_ne!(SortOrder::Asc, SortOrder::Desc);
    }

    // ==================== IndexType Tests ====================

    #[test]
    fn test_index_type_from_str_btree() {
        assert_eq!(IndexType::from_str("btree"), Some(IndexType::BTree));
        assert_eq!(IndexType::from_str("BTree"), Some(IndexType::BTree));
        assert_eq!(IndexType::from_str("BTREE"), Some(IndexType::BTree));
    }

    #[test]
    fn test_index_type_from_str_hash() {
        assert_eq!(IndexType::from_str("hash"), Some(IndexType::Hash));
        assert_eq!(IndexType::from_str("Hash"), Some(IndexType::Hash));
    }

    #[test]
    fn test_index_type_from_str_gist() {
        assert_eq!(IndexType::from_str("gist"), Some(IndexType::Gist));
        assert_eq!(IndexType::from_str("GiST"), Some(IndexType::Gist));
    }

    #[test]
    fn test_index_type_from_str_gin() {
        assert_eq!(IndexType::from_str("gin"), Some(IndexType::Gin));
        assert_eq!(IndexType::from_str("GIN"), Some(IndexType::Gin));
    }

    #[test]
    fn test_index_type_from_str_fulltext() {
        assert_eq!(IndexType::from_str("fulltext"), Some(IndexType::FullText));
        assert_eq!(IndexType::from_str("FullText"), Some(IndexType::FullText));
    }

    #[test]
    fn test_index_type_from_str_unknown() {
        assert_eq!(IndexType::from_str("unknown"), None);
        assert_eq!(IndexType::from_str(""), None);
    }

    #[test]
    fn test_index_type_equality() {
        assert_eq!(IndexType::BTree, IndexType::BTree);
        assert_ne!(IndexType::BTree, IndexType::Hash);
    }

    #[test]
    fn test_index_type_from_str_brin() {
        assert_eq!(IndexType::from_str("brin"), Some(IndexType::Brin));
        assert_eq!(IndexType::from_str("BRIN"), Some(IndexType::Brin));
    }

    #[test]
    fn test_index_type_from_str_hnsw() {
        assert_eq!(IndexType::from_str("hnsw"), Some(IndexType::Hnsw));
        assert_eq!(IndexType::from_str("HNSW"), Some(IndexType::Hnsw));
    }

    #[test]
    fn test_index_type_from_str_ivfflat() {
        assert_eq!(IndexType::from_str("ivfflat"), Some(IndexType::IvfFlat));
        assert_eq!(IndexType::from_str("IVFFLAT"), Some(IndexType::IvfFlat));
    }

    #[test]
    fn test_index_type_is_vector_index() {
        assert!(IndexType::Hnsw.is_vector_index());
        assert!(IndexType::IvfFlat.is_vector_index());
        assert!(!IndexType::BTree.is_vector_index());
        assert!(!IndexType::Gin.is_vector_index());
    }

    #[test]
    fn test_index_type_as_sql() {
        assert_eq!(IndexType::BTree.as_sql(), "BTREE");
        assert_eq!(IndexType::Hash.as_sql(), "HASH");
        assert_eq!(IndexType::Hnsw.as_sql(), "hnsw");
        assert_eq!(IndexType::IvfFlat.as_sql(), "ivfflat");
    }

    // ==================== VectorOps Tests ====================

    #[test]
    fn test_vector_ops_from_str_cosine() {
        assert_eq!(VectorOps::from_str("cosine"), Some(VectorOps::Cosine));
        assert_eq!(
            VectorOps::from_str("vector_cosine_ops"),
            Some(VectorOps::Cosine)
        );
    }

    #[test]
    fn test_vector_ops_from_str_l2() {
        assert_eq!(VectorOps::from_str("l2"), Some(VectorOps::L2));
        assert_eq!(VectorOps::from_str("euclidean"), Some(VectorOps::L2));
        assert_eq!(VectorOps::from_str("vector_l2_ops"), Some(VectorOps::L2));
    }

    #[test]
    fn test_vector_ops_from_str_inner_product() {
        assert_eq!(VectorOps::from_str("ip"), Some(VectorOps::InnerProduct));
        assert_eq!(
            VectorOps::from_str("inner_product"),
            Some(VectorOps::InnerProduct)
        );
        assert_eq!(
            VectorOps::from_str("vector_ip_ops"),
            Some(VectorOps::InnerProduct)
        );
    }

    #[test]
    fn test_vector_ops_as_ops_class() {
        assert_eq!(VectorOps::Cosine.as_ops_class(), "vector_cosine_ops");
        assert_eq!(VectorOps::L2.as_ops_class(), "vector_l2_ops");
        assert_eq!(VectorOps::InnerProduct.as_ops_class(), "vector_ip_ops");
    }

    #[test]
    fn test_vector_ops_as_operator() {
        assert_eq!(VectorOps::Cosine.as_operator(), "<=>");
        assert_eq!(VectorOps::L2.as_operator(), "<->");
        assert_eq!(VectorOps::InnerProduct.as_operator(), "<#>");
    }

    #[test]
    fn test_vector_ops_default() {
        let ops = VectorOps::default();
        assert_eq!(ops, VectorOps::Cosine);
    }

    // ==================== Index with Vector Ops Tests ====================

    #[test]
    fn test_index_with_vector_ops() {
        let idx = Index::new(vec![IndexField::asc("embedding")])
            .with_type(IndexType::Hnsw)
            .with_vector_ops(VectorOps::Cosine)
            .with_hnsw_m(16)
            .with_hnsw_ef_construction(64);

        assert_eq!(idx.index_type, Some(IndexType::Hnsw));
        assert_eq!(idx.vector_ops, Some(VectorOps::Cosine));
        assert_eq!(idx.hnsw_m, Some(16));
        assert_eq!(idx.hnsw_ef_construction, Some(64));
        assert!(idx.is_vector_index());
    }

    #[test]
    fn test_index_with_ivfflat() {
        let idx = Index::new(vec![IndexField::asc("embedding")])
            .with_type(IndexType::IvfFlat)
            .with_vector_ops(VectorOps::L2)
            .with_ivfflat_lists(100);

        assert_eq!(idx.index_type, Some(IndexType::IvfFlat));
        assert_eq!(idx.vector_ops, Some(VectorOps::L2));
        assert_eq!(idx.ivfflat_lists, Some(100));
        assert!(idx.is_vector_index());
    }
}
