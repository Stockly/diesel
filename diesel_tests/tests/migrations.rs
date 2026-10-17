use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};

use crate::schema::TestConnection;
use diesel::QueryResult;
use diesel_migrations::{MigrationHarness, RustMigration};
use diesel_migrations::{RustMigrationSource, TypedMigration};

#[cfg(feature = "postgres")]
mod sql_diagnostics {
    use diesel::migration::MigrationSource;
    use diesel_migrations::{
        EmbeddedMigration, EmbeddedMigrations, EmbeddedName, MigrationDirection, MigrationHarness,
        SqlMigrationError, TomlMetadataWrapper, embed_migrations,
    };

    const BROKEN_UP: EmbeddedMigrations = embed_migrations!("tests/migration_fixtures/broken_up");
    const BROKEN_DOWN: EmbeddedMigrations =
        embed_migrations!("tests/migration_fixtures/broken_down");

    #[diesel_test_helper::test]
    fn embedded_sql_errors_expose_locations_and_preserve_transactions() {
        let conn = &mut crate::schema::connection();
        for (source, direction, file) in [
            (BROKEN_UP, MigrationDirection::Run, "up.sql"),
            (BROKEN_DOWN, MigrationDirection::Revert, "down.sql"),
        ] {
            let migrations =
                <EmbeddedMigrations as MigrationSource<diesel::pg::Pg>>::migrations(&source)
                    .unwrap();
            let migration = &*migrations[0];
            if direction == MigrationDirection::Revert {
                conn.run_migration(migration).unwrap();
            }
            let before = conn.applied_migrations().unwrap();
            let error = match direction {
                MigrationDirection::Run => conn.run_migration(migration).unwrap_err(),
                MigrationDirection::Revert => conn.revert_migration(migration).unwrap_err(),
            };
            let diagnostic = error.downcast_ref::<SqlMigrationError>().unwrap();
            assert_eq!(diagnostic.direction(), direction);
            assert!(diagnostic.path().is_none());
            assert!(diagnostic.is_embedded());
            assert!(diagnostic.source_label().ends_with(file));
            assert_eq!(diagnostic.location().unwrap().line(), 3);
            assert_eq!(diagnostic.location().unwrap().column(), 8);
            assert_eq!(diagnostic.excerpt()[2].text(), "SELECT .;");
            assert!(error.to_string().contains(" (embedded)"));
            assert!(
                std::error::Error::source(diagnostic)
                    .unwrap()
                    .is::<diesel::result::Error>()
            );
            // A failed run/revert must not change recorded migration versions.
            assert_eq!(conn.applied_migrations().unwrap(), before);
        }
    }

    #[diesel_test_helper::test]
    fn server_details_and_hints_without_positions_are_preserved() {
        const SOURCE: EmbeddedMigrations = EmbeddedMigrations::new(&[EmbeddedMigration::new(
            "DO $$ BEGIN RAISE EXCEPTION 'migration failed' USING DETAIL = 'the detail', HINT = 'the hint'; END $$;",
            None,
            EmbeddedName::new("99999999999997_details"),
            TomlMetadataWrapper::new(true),
        )]);
        let conn = &mut crate::schema::connection();
        let error = conn.run_pending_migrations(SOURCE).unwrap_err();
        let diagnostic = error.downcast_ref::<SqlMigrationError>().unwrap();
        assert!(diagnostic.location().is_none());
        assert!(diagnostic.excerpt().is_empty());
        let output = error.to_string();
        assert!(output.contains("99999999999997_details/up.sql (embedded)"));
        assert!(output.contains("DETAIL: the detail\nHINT: the hint"));
    }
}

static GLOBAL_FLAG: AtomicU16 = AtomicU16::new(0);

fn function(_conn: &mut TestConnection) -> QueryResult<()> {
    GLOBAL_FLAG.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

struct Typed(Arc<AtomicBool>);

impl TypedMigration<TestConnection> for Typed {
    fn up(&self, _conn: &mut TestConnection) -> QueryResult<()> {
        self.0.store(true, Ordering::Relaxed);
        Ok(())
    }
}

#[diesel_test_helper::test]
fn test_rust_migrations() {
    let mut source = RustMigrationSource::<TestConnection>::new();
    let callback_called = Arc::new(AtomicBool::new(false));
    let callback_called2 = callback_called.clone();
    let typed_called = Arc::new(AtomicBool::new(false));
    let rust_migration_called = Arc::new(AtomicBool::new(false));
    let rust_migration_called2 = rust_migration_called.clone();

    source
        .add_migration(
            "2026-01-23-173320",
            "test1",
            move |_conn: &mut TestConnection| {
                callback_called.store(true, Ordering::Relaxed);
                Ok(())
            },
        )
        .unwrap();
    source
        .add_migration("2026-01-23-173920", "test2", function)
        .unwrap();
    source
        .add_migration("2026-01-23-174320", "test3", Typed(typed_called.clone()))
        .unwrap();
    let migration = RustMigration::new(move |_conn: &mut TestConnection| {
        rust_migration_called.store(true, Ordering::Relaxed);
        Ok(())
    })
    .with_down(|_| Ok(()))
    .without_transaction();

    source
        .add_migration("2026-01-23-174620", "test4", migration)
        .unwrap();

    #[cfg(not(any(feature = "mysql", feature = "mariadb")))]
    let conn = &mut crate::schema::connection();
    #[cfg(any(feature = "mysql", feature = "mariadb"))]
    let conn = &mut crate::schema::connection_without_transaction();

    assert!(!typed_called.load(Ordering::Relaxed));
    assert!(!callback_called2.load(Ordering::Relaxed));
    assert!(!rust_migration_called2.load(Ordering::Relaxed));
    assert_eq!(GLOBAL_FLAG.load(Ordering::Relaxed), 0);
    let res = conn.run_pending_migrations(source.clone()).map(|c| c.len());
    if cfg!(any(feature = "mysql", feature = "mariadb")) {
        let _ = conn.revert_all_migrations(source);
    }

    let res = res.unwrap();
    assert_eq!(res, 4);

    assert!(typed_called.load(Ordering::Relaxed));
    assert!(callback_called2.load(Ordering::Relaxed));
    assert!(rust_migration_called2.load(Ordering::Relaxed));
    assert_eq!(GLOBAL_FLAG.load(Ordering::Relaxed), 1);
}
