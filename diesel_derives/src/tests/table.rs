use super::FunctionMacro;
use super::expand_with;

#[test]
pub(crate) fn table_1() {
    let input = quote::quote! {
        users {
            id -> Integer,
            name -> Text,
        }
    };
    let name = if cfg!(feature = "postgres") {
        "table_1 (postgres)"
    } else {
        "table_1"
    };

    expand_with(
        &crate::table_proc_inner as &dyn Fn(_) -> _,
        input,
        FunctionMacro(syn::parse_quote!(table)),
        name,
    );
}

#[test]
pub(crate) fn table_with_column_feature_gate() {
    let input = quote::quote! {
        users {
            id -> Integer,
            name -> Text,
            #[cfg(feature = "chrono")]
            created_at -> Timestamp,
        }
    };
    let name = if cfg!(feature = "postgres") {
        "table_with_column_feature_gate (postgres)"
    } else {
        "table_with_column_feature_gate"
    };

    expand_with(
        &crate::table_proc_inner as &dyn Fn(_) -> _,
        input,
        FunctionMacro(syn::parse_quote!(table)),
        name,
    );
}

#[test]
pub(crate) fn table_with_multiple_feature_gated_columns() {
    let input = quote::quote! {
        users {
            id -> Integer,
            name -> Text,
            #[cfg(feature = "chrono")]
            created_at -> Timestamp,
            #[cfg(feature = "uuid")]
            user_uuid -> Uuid,
            #[cfg(feature = "chrono")]
            updated_at -> Timestamp,
        }
    };
    let name = if cfg!(feature = "postgres") {
        "table_with_multiple_feature_gated_columns (postgres)"
    } else {
        "table_with_multiple_feature_gated_columns"
    };

    expand_with(
        &crate::table_proc_inner as &dyn Fn(_) -> _,
        input,
        FunctionMacro(syn::parse_quote!(table)),
        name,
    );
}

#[test]
pub(crate) fn table_with_column_feature_gate_type_check() {
    let input = quote::quote! {
        users {
            id -> Integer,
            name -> Text,
            #[cfg(feature = "test_chrono")]
            created_at -> Timestamp,
        }
    };

    let generated = crate::table_proc_inner(input);
    let generated_str = generated.to_string();

    // This fork replaces the `AllColumns` tuple (and its per-cfg-combination
    // variants) by the `SelectClauseNotSet` marker, so a gated column only
    // carries its `cfg` on its own items, never on the default selection.
    assert!(
        !generated_str.contains("# [cfg (all (not (feature = \"test_chrono\")))]"),
        "Should not generate cfg-combination variants of AllColumns"
    );
    assert!(
        generated_str.contains(
            "# [cfg (feature = \"test_chrono\")] impl diesel :: expression :: Expression for created_at"
        ),
        "Expression impl for created_at should keep its cfg guard"
    );

    assert!(
        generated_str.contains(
            "pub const all_columns : AllColumns = self :: diesel :: query_builder :: SelectClauseNotSet ;"
        ),
        "all_columns should be the SelectClauseNotSet marker"
    );

    assert!(
        generated_str
            .contains("pub type SqlType = < AllColumns as diesel :: Expression > :: SqlType ;"),
        "SqlType should be a single alias derived from the AllColumns type"
    );

    assert!(
        generated_str.contains(
            "pub type AllColumns = self :: diesel :: query_builder :: SelectClauseNotSet ;"
        ),
        "AllColumns alias should be the SelectClauseNotSet marker"
    );

    assert!(
        generated_str.contains("type AllColumns = AllColumns ;"),
        "Table::AllColumns associated type should reference the AllColumns alias"
    );
}

#[test]
pub(crate) fn table_with_multiple_feature_gated_columns_type_check() {
    let input = quote::quote! {
        users {
            id -> Integer,
            name -> Text,
            #[cfg(feature = "test_chrono")]
            created_at -> Timestamp,
            #[cfg(feature = "test_uuid")]
            user_uuid -> Uuid,
            #[cfg(feature = "test_chrono")]
            updated_at -> Timestamp,
        }
    };

    let generated = crate::table_proc_inner(input);
    let generated_str = generated.to_string();

    // Same fork behaviour as `table_with_column_feature_gate_type_check`: no
    // cfg-combination variants of `AllColumns`, the `cfg` stays on each column.
    assert!(
        !generated_str.contains(
            "# [cfg (all (not (feature = \"test_chrono\") , not (feature = \"test_uuid\")))]"
        ),
        "Should not generate cfg-combination variants of AllColumns"
    );
    assert!(
        generated_str.contains(
            "# [cfg (feature = \"test_chrono\")] impl diesel :: expression :: Expression for created_at"
        ),
        "Expression impl for created_at should keep its cfg guard"
    );
    assert!(
        generated_str.contains(
            "# [cfg (feature = \"test_uuid\")] impl diesel :: expression :: Expression for user_uuid"
        ),
        "Expression impl for user_uuid should keep its cfg guard"
    );
    assert!(
        generated_str.contains(
            "# [cfg (feature = \"test_chrono\")] impl diesel :: expression :: Expression for updated_at"
        ),
        "Expression impl for updated_at should keep its cfg guard"
    );

    assert!(
        generated_str.contains(
            "pub const all_columns : AllColumns = self :: diesel :: query_builder :: SelectClauseNotSet ;"
        ),
        "all_columns should be the SelectClauseNotSet marker"
    );

    assert!(
        generated_str
            .contains("pub type SqlType = < AllColumns as diesel :: Expression > :: SqlType ;"),
        "SqlType should be a single alias derived from the AllColumns type"
    );

    assert!(
        generated_str.contains(
            "pub type AllColumns = self :: diesel :: query_builder :: SelectClauseNotSet ;"
        ),
        "AllColumns alias should be the SelectClauseNotSet marker"
    );

    assert!(
        generated_str.contains("type AllColumns = AllColumns ;"),
        "Table::AllColumns associated type should reference the AllColumns alias"
    );
}

#[test]
pub(crate) fn table_with_feature_gate_valid_grouping_star_check() {
    let input = quote::quote! {
        users {
            id -> Integer,
            name -> Text,
            #[cfg(feature = "test_feature")]
            extra -> Text,
        }
    };

    let generated = crate::table_proc_inner(input);
    let generated_str = generated.to_string();

    assert!(
        generated_str
            .contains("super :: AllColumns : diesel :: expression :: ValidGrouping < __GB >"),
        "ValidGrouping for star should reference super::AllColumns rather than an inlined column tuple"
    );
    assert!(
        generated_str.contains(
            "< super :: AllColumns as diesel :: expression :: ValidGrouping < __GB >> :: IsAggregate"
        ),
        "IsAggregate projection should be on super::AllColumns"
    );
}

#[test]
pub(crate) fn table_with_complex_feature_gates() {
    let input = quote::quote! {
        users {
            id -> Integer,
            name -> Text,
            #[cfg(all(feature = "postgres", feature = "chrono"))]
            pg_created_at -> Timestamptz,
            #[cfg(any(feature = "sqlite", feature = "mysql"))]
            simple_created_at -> Timestamp,
            #[cfg(not(feature = "production"))]
            debug_info -> Text,
            #[cfg(all(feature = "advanced", not(feature = "lite")))]
            advanced_field -> Text,
        }
    };

    let generated = crate::table_proc_inner(input);
    let generated_str = generated.to_string();

    assert!(
        generated_str.contains("# [cfg (all (feature = \"postgres\" , feature = \"chrono\"))]"),
        "Should preserve all(feature, feature) cfg condition"
    );
    assert!(
        generated_str.contains("# [cfg (any (feature = \"sqlite\" , feature = \"mysql\"))]"),
        "Should preserve any(feature, feature) cfg condition"
    );
    assert!(
        generated_str.contains("# [cfg (not (feature = \"production\"))]"),
        "Should preserve not(feature) cfg condition"
    );
    assert!(
        generated_str.contains("# [cfg (all (feature = \"advanced\" , not (feature = \"lite\")))]"),
        "Should preserve all(feature, not(feature)) cfg condition"
    );

    assert!(
        generated_str.contains("pub struct pg_created_at"),
        "pg_created_at column struct should be generated"
    );
    assert!(
        generated_str.contains("pub struct simple_created_at"),
        "simple_created_at column struct should be generated"
    );
    assert!(
        generated_str.contains("pub struct debug_info"),
        "debug_info column struct should be generated"
    );
    assert!(
        generated_str.contains("pub struct advanced_field"),
        "advanced_field column struct should be generated"
    );

    assert!(
        generated_str.contains("# [cfg (all (feature = \"postgres\" , feature = \"chrono\"))] impl diesel :: expression :: Expression for pg_created_at"),
        "Expression impl for pg_created_at should have cfg guard"
    );
    assert!(
        generated_str.contains("# [cfg (any (feature = \"sqlite\" , feature = \"mysql\"))] impl diesel :: expression :: Expression for simple_created_at"),
        "Expression impl for simple_created_at should have cfg guard"
    );
    assert!(
        generated_str.contains("# [cfg (not (feature = \"production\"))] impl diesel :: expression :: Expression for debug_info"),
        "Expression impl for debug_info should have cfg guard"
    );
    assert!(
        generated_str.contains("# [cfg (all (feature = \"advanced\" , not (feature = \"lite\")))] impl diesel :: expression :: Expression for advanced_field"),
        "Expression impl for advanced_field should have cfg guard"
    );

    // This fork has no `AllColumns` tuple, hence no cfg-combination variants.
    assert!(
        generated_str.contains(
            "pub const all_columns : AllColumns = self :: diesel :: query_builder :: SelectClauseNotSet ;"
        ),
        "all_columns should be the SelectClauseNotSet marker"
    );
    assert!(
        !generated_str.contains("not (all (feature = \"postgres\" , feature = \"chrono\"))"),
        "Should not generate negated combinatorial cfg conditions"
    );
    assert!(
        !generated_str.contains("not (any (feature = \"sqlite\" , feature = \"mysql\"))"),
        "Should not generate negated combinatorial any() conditions"
    );
}

#[test]
pub(crate) fn table_with_complex_feature_gates_cross_column_impls() {
    let input = quote::quote! {
        users {
            id -> Integer,
            #[cfg(all(feature = "a", feature = "b"))]
            col_ab -> Text,
            #[cfg(any(feature = "c", feature = "d"))]
            col_cd -> Text,
        }
    };

    let generated = crate::table_proc_inner(input);
    let generated_str = generated.to_string();

    assert!(
        generated_str.contains("# [cfg (any (feature = \"c\" , feature = \"d\"))] # [cfg (all (feature = \"a\" , feature = \"b\"))] impl diesel :: expression :: IsContainedInGroupBy < col_cd > for col_ab"),
        "IsContainedInGroupBy<col_cd> for col_ab should have cfg attrs from both columns"
    );
    assert!(
        generated_str.contains("# [cfg (any (feature = \"c\" , feature = \"d\"))] # [cfg (all (feature = \"a\" , feature = \"b\"))] impl diesel :: expression :: IsContainedInGroupBy < col_ab > for col_cd"),
        "IsContainedInGroupBy<col_ab> for col_cd should have cfg attrs from both columns"
    );
}
