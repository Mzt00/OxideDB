use std::fmt;
#[derive(Debug)]
pub enum DbError{
    Io(std::io::Error),
    SyntaxError(String),
    TableNotFound(String),
    TableAlreadyExists(String),
    ColumnNotFound(String),
    DuplicateColumn(String),
    TypeMismatch{
        expected: String,
        actual: String,
    },
    NullConstraintViolation(String),
    InvalidSchema(String),
    InvalidRow(String),
    DuplicateKey(String),
    CorruptData(String),
    RowTooLarge{
        row_size: usize,
        page_size: usize,
    },
    PageError(String),
    InvalidPageId(u32),
    CatalogError(u32),
    UnsupportedString(String), //for features which are not available yet
}

pub type Result<T> = std::result::Result<T,DbError>;
impl fmt::Display for DbError{
    fn fmt(&self, f: &mut fmt::Formatter<'_>)->fmt::Result{
        match self{
            DbError::Io(error)=>{
                write!(f, "I/O Error: {}",error)
            }
            DbError::SyntaxError(message) => {
                write!(f, "SQL syntax error: {}", message)
            }
            DbError::TableAlreadyExists(name) => {
                write!(f, "table already exists: '{}'", name)
            }
            DbError::TableNotFound(name) => {
                write!(f, "table not found: '{}'", name)
            }
            DbError::ColumnNotFound(name) => {
                write!(f, "column not found: '{}'", name)
            }
            DbError::DuplicateColumn(name) => {
                write!(f, "duplicate column: '{}'", name)
            }
            DbError::TypeMismatch { expected, actual } => {
                write!(
                    f,
                    "type mismatch: expected {}, got {}",
                    expected, actual
                )
            }
            DbError::NullConstraintViolation(column_name) => {
                write!(
                    f,
                    "column '{}' does not allow NULL values",
                    column_name
                )
            }
            DbError::InvalidSchema(reason) => {
                write!(f, "invalid schema: {}", reason)
            }
            DbError::InvalidRow(reason) => {
                write!(f, "invalid row: {}", reason)
            }
            DbError::DuplicateKey(key) => {
                write!(f, "duplicate primary key: {}", key)
            }
            DbError::CorruptData(reason) => {
                write!(f, "corrupt database data: {}", reason)
            }
            DbError::RowTooLarge {
                row_size,
                page_size,
            } => {
                write!(
                    f,
                    "row is too large: row requires {} bytes, but page size is {} bytes",
                    row_size, page_size
                )
            }
            DbError::PageError(reason) => {
                write!(f, "page error: {}", reason)
            }
            DbError::InvalidPageId(page_id) => {
                write!(f, "invalid page ID: {}", page_id)
            }
            DbError::CatalogError(reason) => {
                write!(f, "catalog error: {}", reason)
            }
            DbError::Unsupported(feature) => {
                write!(f, "unsupported feature: {}", feature)
            }
            

        }
    }
}
impl std::error::Error for DbError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            DbError::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for DbError {
    fn from(error: std::io::Error) -> Self {
        DbError::Io(error)
    }
}