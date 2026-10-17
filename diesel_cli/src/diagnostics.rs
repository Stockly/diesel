use std::io::{self, IsTerminal, Write};

use anstream::{AutoStream, ColorChoice};
use diesel_migrations::SqlMigrationError;

use crate::errors::Error;

pub(crate) fn print_error(error: &Error) {
    let stderr = io::stderr();
    // Never emit styling to redirected stderr, even if color is forced in the
    // environment. Library Display remains independent of all environment state.
    let choice = if stderr.is_terminal()
        && !std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
        && std::env::var_os("TERM").is_none_or(|term| term != "dumb")
    {
        ColorChoice::Auto
    } else {
        ColorChoice::Never
    };
    let mut stream = AutoStream::new(stderr.lock(), choice);
    let color = matches!(
        stream.current_choice(),
        ColorChoice::Always | ColorChoice::AlwaysAnsi
    );
    // A closed stderr must not turn an ordinary CLI failure into a panic.
    let _ = writeln!(stream, "{}", format_error(error, color));
}

fn format_error(error: &Error, color: bool) -> String {
    if color
        && let Error::MigrationError(inner) = error
        && let Some(diagnostic) = inner.downcast_ref::<SqlMigrationError>()
    {
        format!(
            "Failed to run migrations: {}",
            diagnostic.display_with_color()
        )
    } else {
        error.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn other_errors_keep_their_existing_format() {
        let error = Error::DatabaseUrlMissing;
        assert_eq!(format_error(&error, false), error.to_string());
        assert_eq!(format_error(&error, true), error.to_string());
    }

    #[test]
    #[cfg(feature = "sqlite")]
    fn sql_errors_have_explicit_plain_and_colored_renderings() {
        use diesel::Connection;
        use diesel_migrations::{FileBasedMigrations, MigrationHarness};

        let directory = tempfile::tempdir().unwrap();
        let migration = directory.path().join("123_broken");
        std::fs::create_dir(&migration).unwrap();
        std::fs::write(migration.join("up.sql"), "SELECT .;").unwrap();
        let source = FileBasedMigrations::from_path(directory.path()).unwrap();
        let mut conn = diesel::SqliteConnection::establish(":memory:").unwrap();
        let error = Error::MigrationError(conn.run_pending_migrations(source).unwrap_err());
        let plain = format_error(&error, false);
        let color = format_error(&error, true);
        assert_eq!(plain, error.to_string());
        assert!(!plain.contains('\x1b'));
        assert!(color.contains('\x1b'));
        assert!(color.starts_with("Failed to run migrations: Failed to run 123_broken with: "));
        // Styling changes presentation only, including the outer CLI prefix.
        let mut stream = AutoStream::new(Vec::new(), ColorChoice::Never);
        write!(stream, "{color}").unwrap();
        assert_eq!(String::from_utf8(stream.into_inner()).unwrap(), plain);
    }
}
