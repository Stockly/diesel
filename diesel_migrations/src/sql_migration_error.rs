//! Source diagnostics for failures while executing SQL migrations.

use std::borrow::Cow;
use std::fmt;
use std::path::{Path, PathBuf};

use anstyle::{AnsiColor, Style};
use diesel::result::{DatabaseErrorInformation, Error};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

const CONTEXT_LINES: usize = 2;
// Reserve a column on each side for an omission marker.
const WINDOW_WIDTH: usize = 118;

/// The direction in which a failed SQL migration was executed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDirection {
    /// Executing `up.sql`.
    Run,
    /// Executing `down.sql`.
    Revert,
}

impl MigrationDirection {
    fn file_name(self) -> &'static str {
        match self {
            Self::Run => "up.sql",
            Self::Revert => "down.sql",
        }
    }
}

/// A position in the original SQL, before tab expansion or excerpt cropping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationErrorLocation {
    line: usize,
    column: usize,
}

impl MigrationErrorLocation {
    /// The one-based source line number.
    pub fn line(self) -> usize {
        self.line
    }

    /// The one-based source column, measured in Unicode scalar values, not bytes
    /// or terminal display columns. A tab counts as one character.
    pub fn column(self) -> usize {
        self.column
    }
}

/// A bounded, display-ready line from the failed migration.
#[derive(Debug, PartialEq, Eq)]
pub struct MigrationErrorLine {
    number: usize,
    text: String,
}

impl MigrationErrorLine {
    /// The one-based line number in the original SQL.
    pub fn number(&self) -> usize {
        self.number
    }

    /// Display-ready SQL, with four-column tab stops, escaped control characters,
    /// and `…` marking horizontal omissions. At most 120 display columns.
    /// This is an excerpt, not SQL suitable for execution.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A failure while executing a file-based or embedded SQL migration.
///
/// The boxed error returned by the migration harness can be downcast to this
/// type. [`Display`](fmt::Display) includes a plain-text source diagnostic;
/// [`Self::display_with_color`] explicitly opts into ANSI styling.
///
/// Only a bounded excerpt is retained, not the complete SQL. Locations and
/// excerpts are available when the backend supplies a valid statement position
/// (currently PostgreSQL). Other failures still identify the SQL source.
/// The original Diesel error is available through [`Self::error`] and
/// [`std::error::Error::source`].
///
/// ```
/// use diesel_migrations::SqlMigrationError;
///
/// fn report(error: &(dyn std::error::Error + 'static)) {
///     if let Some(sql_error) = error.downcast_ref::<SqlMigrationError>() {
///         if let Some(location) = sql_error.location() {
///             eprintln!("{}:{}:{}", sql_error.source_label(), location.line(), location.column());
///         }
///         // The original error remains available for application-specific handling.
///         let _database_error = sql_error.error();
///     }
/// }
/// ```
#[derive(Debug)]
pub struct SqlMigrationError {
    migration_name: String,
    direction: MigrationDirection,
    path: Option<PathBuf>,
    source_label: String,
    location: Option<MigrationErrorLocation>,
    excerpt: Vec<MigrationErrorLine>,
    caret_column: Option<usize>,
    error: Error,
}

impl SqlMigrationError {
    pub(crate) fn new(
        migration_name: String,
        direction: MigrationDirection,
        path: Option<&Path>,
        sql: &str,
        error: Error,
    ) -> Self {
        let source_label = match path {
            Some(path) => path.display().to_string(),
            None => format!("{migration_name}/{}", direction.file_name()),
        };
        let diagnostic = match &error {
            Error::DatabaseError(_, info) => {
                info.statement_position().and_then(|p| excerpt(sql, p))
            }
            _ => None,
        };
        let (location, excerpt, caret_column) = match diagnostic {
            Some((location, lines, caret)) => (Some(location), lines, Some(caret)),
            None => (None, Vec::new(), None),
        };
        Self {
            migration_name,
            direction,
            path: path.map(Path::to_path_buf),
            source_label,
            location,
            excerpt,
            caret_column,
            error,
        }
    }

    /// The name of the failed migration, including its version.
    pub fn migration_name(&self) -> &str {
        &self.migration_name
    }

    /// Whether this failure occurred while running or reverting the migration.
    pub fn direction(&self) -> MigrationDirection {
        self.direction
    }

    /// The supplied filesystem path, or `None` for an embedded migration.
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The displayed path, or `<migration-name>/up.sql` or `down.sql` for an
    /// embedded migration. Non-UTF-8 filesystem paths are represented lossily;
    /// [`Self::path`] retains the original path.
    pub fn source_label(&self) -> &str {
        &self.source_label
    }

    /// Whether the SQL was embedded in the application binary.
    pub fn is_embedded(&self) -> bool {
        self.path.is_none()
    }

    /// The error location, if the backend supplied a valid statement position.
    pub fn location(&self) -> Option<MigrationErrorLocation> {
        self.location
    }

    /// Up to five display-ready source lines, using a common horizontal window.
    /// Empty when no valid location is available.
    pub fn excerpt(&self) -> &[MigrationErrorLine] {
        &self.excerpt
    }

    /// The one-based display column of the caret in the error line's excerpt,
    /// including any leading omission marker but excluding the line-number gutter.
    /// Positions within a grapheme cluster or ligature point to its start.
    pub fn caret_column(&self) -> Option<usize> {
        self.caret_column
    }

    /// The original database or query execution error.
    pub fn error(&self) -> &Error {
        &self.error
    }

    /// Explicitly render this diagnostic with ANSI colors. This does not inspect
    /// the terminal or environment; callers must decide whether color is suitable.
    /// Ordinary [`Display`](fmt::Display) always uses plain text.
    pub fn display_with_color(&self) -> impl fmt::Display + '_ {
        DiagnosticDisplay {
            error: self,
            color: true,
        }
    }

    fn database_information(&self) -> Option<&(dyn DatabaseErrorInformation + Send + Sync)> {
        match &self.error {
            Error::DatabaseError(_, info) => Some(&**info),
            _ => None,
        }
    }
}

impl std::error::Error for SqlMigrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl fmt::Display for SqlMigrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        DiagnosticDisplay {
            error: self,
            color: false,
        }
        .fmt(f)
    }
}

struct DiagnosticDisplay<'a> {
    error: &'a SqlMigrationError,
    color: bool,
}

impl fmt::Display for DiagnosticDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let error = self.error;
        let red = if self.color {
            AnsiColor::Red.on_default().bold()
        } else {
            Style::new()
        };
        let blue = if self.color {
            AnsiColor::Blue.on_default().bold()
        } else {
            Style::new()
        };
        write!(
            f,
            "Failed to run {} with: {}",
            error.migration_name, error.error
        )?;
        if let Some(info) = error.database_information()
            && info
                .message()
                .ends_with("cannot run inside a transaction block")
        {
            write!(
                f,
                " (see https://docs.diesel.rs/{}.x/diesel_migrations/struct.FileBasedMigrations.html#transactions)",
                env!("CARGO_PKG_VERSION")
                    .rsplit_once('.')
                    .expect("semver version")
                    .0
            )?;
        }
        write!(
            f,
            "\n {blue}-->{blue:#} {}",
            escape_controls(&error.source_label)
        )?;
        if let Some(location) = error.location {
            write!(f, ":{}:{}", location.line, location.column)?;
        }
        if error.is_embedded() {
            write!(f, " (embedded)")?;
        }
        if let Some(location) = error.location {
            let width = error
                .excerpt
                .last()
                .map_or(1, |line| line.number.to_string().len());
            write!(f, "\n {:width$} {blue}|{blue:#}", "")?;
            for line in &error.excerpt {
                write!(f, "\n {blue}{:width$} |{blue:#} {}", line.number, line.text)?;
                if line.number == location.line {
                    let padding = error.caret_column.unwrap_or(1) - 1;
                    write!(
                        f,
                        "\n {:width$} {blue}|{blue:#} {:padding$}{red}^{red:#}",
                        "", ""
                    )?;
                }
            }
        }
        if let Some(info) = error.database_information() {
            if let Some(details) = info.details() {
                write!(f, "\nDETAIL: {details}")?;
            }
            if let Some(hint) = info.hint() {
                write!(f, "\nHINT: {hint}")?;
            }
        }
        Ok(())
    }
}

fn escape_controls(text: &str) -> Cow<'_, str> {
    if text.chars().any(char::is_control) {
        Cow::Owned(
            text.chars()
                .map(|c| {
                    if c.is_control() {
                        c.escape_default().to_string()
                    } else {
                        c.to_string()
                    }
                })
                .collect(),
        )
    } else {
        Cow::Borrowed(text)
    }
}

fn display_grapheme(grapheme: &str, column: usize) -> (Cow<'_, str>, usize) {
    if grapheme == "\t" {
        let width = 4 - column % 4;
        (Cow::Owned(" ".repeat(width)), width)
    } else {
        let text = escape_controls(grapheme);
        let width = UnicodeWidthStr::width(text.as_ref());
        (text, width)
    }
}

// Some script-specific ligatures (for example Arabic lam-alef) span grapheme
// boundaries. Keep them together so cropping and caret placement use the same
// width as the rendered text. Tabs and escaped controls interrupt shaping.
fn display_units(line: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut graphemes = line.grapheme_indices(true).peekable();
    std::iter::from_fn(move || {
        let (start, first) = graphemes.next()?;
        let mut end = start + first.len();
        let mut width = UnicodeWidthStr::width(first);
        while let Some(&(next_start, next)) = graphemes.peek() {
            if line[start..end].chars().any(char::is_control) || next.chars().any(char::is_control)
            {
                break;
            }
            let next_end = next_start + next.len();
            let joined_width = UnicodeWidthStr::width(&line[start..next_end]);
            if joined_width == width + UnicodeWidthStr::width(next) {
                break;
            }
            graphemes.next();
            end = next_end;
            width = joined_width;
        }
        Some((start, &line[start..end]))
    })
}

fn source_lines(sql: &str) -> impl Iterator<Item = &str> {
    sql.split_inclusive('\n')
        .map(|line| match line.strip_suffix('\n') {
            Some(line) => line.strip_suffix('\r').unwrap_or(line),
            None => line,
        })
        .chain((sql.is_empty() || sql.ends_with('\n')).then_some(""))
}

fn excerpt(
    sql: &str,
    position: i32,
) -> Option<(MigrationErrorLocation, Vec<MigrationErrorLine>, usize)> {
    let offset = usize::try_from(position).ok()?.checked_sub(1)?;
    // PostgreSQL uses one-based character offsets into the entire submitted batch.
    // Include the boundary after the last character for end-of-input errors.
    let byte = sql
        .char_indices()
        .map(|(byte, _)| byte)
        .chain(std::iter::once(sql.len()))
        .nth(offset)?;
    let prefix = &sql[..byte];
    let line_number = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |byte| byte + 1);
    let location = MigrationErrorLocation {
        line: line_number,
        column: sql[line_start..byte].chars().count() + 1,
    };
    let error_line = source_lines(sql).nth(line_number - 1)?;
    let mut caret = 0;
    for (index, grapheme) in display_units(error_line) {
        if index + grapheme.len() > byte - line_start {
            break;
        }
        caret += display_grapheme(grapheme, caret).1;
    }
    let window_start = if caret >= WINDOW_WIDTH {
        caret.saturating_sub(WINDOW_WIDTH / 2)
    } else {
        0
    };
    let first_line = line_number.saturating_sub(CONTEXT_LINES).max(1);
    let lines = source_lines(sql)
        .enumerate()
        .skip(first_line - 1)
        .take(line_number + CONTEXT_LINES - first_line + 1)
        .map(|(index, line)| MigrationErrorLine {
            number: index + 1,
            text: crop_line(line, window_start),
        })
        .collect();
    Some((
        location,
        lines,
        caret - window_start + usize::from(window_start > 0) + 1,
    ))
}

fn crop_line(line: &str, window_start: usize) -> String {
    let window_end = window_start + WINDOW_WIDTH;
    let mut result = String::new();
    if window_start > 0 {
        result.push(if line.is_empty() { ' ' } else { '\u{2026}' });
    }
    let mut column = 0;
    for (_, grapheme) in display_units(line) {
        let (text, width) = display_grapheme(grapheme, column);
        let next = column + width;
        if column >= window_start && next <= window_end && column < window_end {
            result.push_str(&text);
        } else if next > window_start && column < window_end {
            // Do not split a wide grapheme at either edge of the common window.
            result.push_str(&" ".repeat(next.min(window_end) - column.max(window_start)));
        }
        column = next;
        if column > window_end {
            result.push('\u{2026}');
            break;
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use diesel::result::DatabaseErrorKind;
    use std::error::Error as _;

    struct Information {
        position: Option<i32>,
        message: &'static str,
        details: Option<&'static str>,
        hint: Option<&'static str>,
    }

    impl DatabaseErrorInformation for Information {
        fn message(&self) -> &str {
            self.message
        }
        fn details(&self) -> Option<&str> {
            self.details
        }
        fn hint(&self) -> Option<&str> {
            self.hint
        }
        fn table_name(&self) -> Option<&str> {
            None
        }
        fn column_name(&self) -> Option<&str> {
            None
        }
        fn constraint_name(&self) -> Option<&str> {
            None
        }
        fn statement_position(&self) -> Option<i32> {
            self.position
        }
    }

    fn diagnostic(sql: &str, position: Option<i32>) -> SqlMigrationError {
        SqlMigrationError::new(
            "123_create_users".into(),
            MigrationDirection::Run,
            Some(Path::new("migrations/123_create_users/up.sql")),
            sql,
            Error::DatabaseError(
                DatabaseErrorKind::Unknown,
                Box::new(Information {
                    position,
                    message: "syntax error",
                    details: None,
                    hint: None,
                }),
            ),
        )
    }

    fn position_of(sql: &str, needle: &str) -> i32 {
        i32::try_from(sql[..sql.find(needle).unwrap()].chars().count() + 1).unwrap()
    }

    #[test]
    fn plain_diagnostic_has_bounded_context_and_original_error() {
        let error = diagnostic("first\nsecond\nthird\nbad\nfifth\nsixth\nseventh", Some(20));
        assert_eq!(
            error.location(),
            Some(MigrationErrorLocation { line: 4, column: 1 })
        );
        assert_eq!(
            error.to_string(),
            concat!(
                "Failed to run 123_create_users with: syntax error\n",
                " --> migrations/123_create_users/up.sql:4:1\n",
                "   |\n",
                " 2 | second\n",
                " 3 | third\n",
                " 4 | bad\n",
                "   | ^\n",
                " 5 | fifth\n",
                " 6 | sixth"
            )
        );
        assert_eq!(error.migration_name(), "123_create_users");
        assert_eq!(error.direction(), MigrationDirection::Run);
        assert_eq!(
            error.path(),
            Some(Path::new("migrations/123_create_users/up.sql"))
        );
        assert!(!error.is_embedded());
        assert!(std::ptr::eq(
            error.source().unwrap().downcast_ref::<Error>().unwrap(),
            error.error()
        ));
        let colored = error.display_with_color().to_string();
        assert!(colored.contains("\x1b["));
        assert!(colored.contains("syntax error"));
        assert!(!error.to_string().contains('\x1b'));
    }

    #[test]
    fn character_coordinates_are_distinct_from_display_coordinates() {
        let sql = "-- \u{e9}\r\n\tSELECT \u{754c}e\u{301}X";
        let error = diagnostic(sql, Some(position_of(sql, "X")));
        assert_eq!(
            error.location(),
            Some(MigrationErrorLocation {
                line: 2,
                column: 12
            })
        );
        assert_eq!(error.caret_column(), Some(15));
        assert_eq!(error.excerpt()[1].text(), "    SELECT \u{754c}e\u{301}X");
        assert!(!error.to_string().contains('\r'));

        // A scalar within a grapheme still reports its original source column,
        // but the visual caret points at the beginning of that grapheme.
        let error = diagnostic(sql, Some(position_of(sql, "\u{301}")));
        assert_eq!(error.location().unwrap().column(), 11);
        assert_eq!(error.caret_column(), Some(14));

        let sql = "\u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}X";
        let error = diagnostic(sql, Some(position_of(sql, "X")));
        assert_eq!(error.caret_column(), Some(3));
    }

    #[test]
    fn eof_and_line_boundaries_have_valid_carets() {
        for (sql, position, line, column, caret) in [
            ("SELECT", 7, 1, 7, 7),
            ("SELECT\n", 8, 2, 1, 1),
            ("SELECT\r\n", 9, 2, 1, 1),
            ("SELECT\r\n", 8, 1, 8, 7),
            ("", 1, 1, 1, 1),
            ("X\nY", 3, 2, 1, 1),
        ] {
            let error = diagnostic(sql, Some(position));
            assert_eq!(
                error.location(),
                Some(MigrationErrorLocation { line, column }),
                "{sql:?}"
            );
            assert_eq!(error.caret_column(), Some(caret), "{sql:?}");
            assert!(error.to_string().contains('^'));
        }
    }

    #[test]
    fn missing_or_invalid_positions_do_not_retain_sql() {
        for position in [None, Some(0), Some(-1), Some(5), Some(i32::MAX)] {
            let error = diagnostic("SQL", position);
            assert!(error.location().is_none());
            assert!(error.excerpt().is_empty());
            assert!(error.caret_column().is_none());
            assert_eq!(
                error.to_string(),
                "Failed to run 123_create_users with: syntax error\n --> migrations/123_create_users/up.sql"
            );
        }
    }

    #[test]
    fn cropped_lines_share_a_window_and_do_not_split_graphemes() {
        let long_line = format!(
            "{}X{}",
            "\u{754c}".repeat(100),
            "\u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}".repeat(100)
        );
        let sql = format!("short\n{long_line}\n{long_line}");
        let error = diagnostic(&sql, Some(position_of(&sql, "X")));
        assert_eq!(error.excerpt()[0].text(), "\u{2026}");
        assert_eq!(error.excerpt()[1].text(), error.excerpt()[2].text());
        for line in error.excerpt() {
            assert!(UnicodeWidthStr::width(line.text()) <= 120);
        }
        let text = error.excerpt()[1].text();
        assert!(text.starts_with('\u{2026}') && text.ends_with('\u{2026}'));
        assert_eq!(
            UnicodeWidthStr::width(&text[..text.find('X').unwrap()]) + 1,
            error.caret_column().unwrap()
        );
        assert!(text.contains("\u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}"));
        assert!(!text.ends_with('\u{200d}'));
    }

    #[test]
    fn controls_are_visible_and_tab_stops_follow_display_width() {
        let sql = "\u{754c}\t\x1bX";
        let error = diagnostic(sql, Some(position_of(sql, "X")));
        assert_eq!(error.excerpt()[0].text(), "\u{754c}  \\u{1b}X");
        assert_eq!(error.caret_column(), Some(11));
        assert!(!error.to_string().contains('\x1b'));
    }

    #[test]
    fn ligatures_across_grapheme_boundaries_keep_the_caret_aligned() {
        // Arabic lam-alef and the Hebrew alef-lamed ligature each occupy one
        // display column despite containing multiple grapheme clusters.
        for ligature in ["\u{644}\u{627}", "\u{5d0}\u{200d}\u{5dc}"] {
            for count in [1, 200] {
                let sql = format!("{}X", ligature.repeat(count));
                let error = diagnostic(&sql, Some(position_of(&sql, "X")));
                let text = error.excerpt()[0].text();
                assert_eq!(UnicodeWidthStr::width(ligature), 1);
                assert_eq!(
                    error.caret_column().unwrap(),
                    UnicodeWidthStr::width(&text[..text.find('X').unwrap()]) + 1
                );
                assert!(UnicodeWidthStr::width(text) <= 120);
            }
        }
    }

    #[test]
    fn embedded_fallback_preserves_details_hints_and_transaction_guidance() {
        let error = SqlMigrationError::new(
            "123_index".into(),
            MigrationDirection::Revert,
            None,
            "CREATE INDEX CONCURRENTLY idx ON users (id);",
            Error::DatabaseError(
                DatabaseErrorKind::Unknown,
                Box::new(Information {
                    position: None,
                    message: "CREATE INDEX CONCURRENTLY cannot run inside a transaction block",
                    details: Some("Database detail\ncontinued"),
                    hint: Some("Database hint"),
                }),
            ),
        );
        let output = error.to_string();
        assert!(error.is_embedded());
        assert_eq!(error.direction(), MigrationDirection::Revert);
        assert_eq!(error.source_label(), "123_index/down.sql");
        assert!(output.contains("struct.FileBasedMigrations.html#transactions)"));
        assert!(output.contains(" --> 123_index/down.sql (embedded)\nDETAIL: Database detail\ncontinued\nHINT: Database hint"));
        assert!(error.excerpt().is_empty());
    }
}
