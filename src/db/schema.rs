use super::error::{DbError, Result};
use super::value::{DataType, Value};

#[derive(Debug, Clone, PartialEq, Eq)]

pub struct Column{
    pub name: String,
    pub data_type: DataType,
    pub nullable: bool,
    pub primary_key: bool,
}

impl Column{
    pub fn new(
        name: String,
        data_type: DataType,
        nullable: bool,
        primary_key:bool,
    )->Self{
        Self { name, data_type, nullable, primary_key }
    }

    pub fn name(&self)-> &str{
        &self.name
    }

    pub fn data_type(&self)->DataType{
        self.data_type
    }

    pub fn is_nullable(&self)->bool{
        self.nullable
    }

    pub fn is_primary_key(&self)->bool{
        self.primary_key
    }

    pub fn validate_value(&self,value: &Value)->Result<()>{
        if value.is_null(){
            if self.nullable{
                return Ok(());
            }
            return Err(DbError::NullConstraintViolation(self.name.clone()));
        }
        if !value.match_type(self.data_type){
            let actual = match value.data_type(){
                Some(data_type)=>data_type.to_string(),
                None => "Null".to_string(),
            };
            return Err(DbError::TypeMismatch { expected: self.data_type.to_string(), actual, });
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]

pub struct Schema{
    table_name: String,
    columns: Vec<Column>,
    primary_key_index: usize,
}

impl Schema{
    pub fn new(
        table_name:String,
        columns: Vec<Column>,
    )-> Result<Self>{
        if table_name.trim().is_empty(){
            return Err(DbError::InvalidSchema("table name cannot be empty".to_string(),));
        }
        if columns.is_empty(){
            return Err(DbError::InvalidSchema("table must contain atleast one column".to_string(),));
        }
        for column in &columns{
            if column.name.trim().is_empty(){
                return Err(DbError::InvalidSchema("column name cannot be empty".to_string(),));
            }
        }
        for i in 0..columns.len(){
            for j in (i+1)..columns.len(){
                if columns[i].name == columns[j].name{
                    return Err(DbError::DuplicateColumn(columns[i].name.clone(),));
                }
            }
        }
        let mut primary_key_index: Option<usize> = None;
        for (index,column) in columns.iter().enumerate(){
            if !column.primary_key{
                continue;
            }
            if primary_key_index.is_some(){
                return Err(DbError::InvalidSchema("table cannot have more than one primary key".to_string(),));
            }

            if column.nullable{
                return Err(DbError::InvalidSchema("primary key cannot be null".to_string(),));
            }
            //only integer and text can be primary keys for now
            if column.data_type != DataType::Integer && column.data_type != DataType::Text{
                return Err(DbError::InvalidSchema("primary key must be INTEGER or TEXT".to_string(),));
            }
            primary_key_index = Some(index);
        }
        let primary_key_index = match primary_key_index{
            Some(index) =>index,
            None=>{
                return Err(DbError::InvalidSchema("table must have exactly one primary key".to_string(),));
            }
        };
        Ok(Self{
            table_name,
            columns,
            primary_key_index,
        })

    }

    pub fn table_name(&self)->&str{
        return &self.table_name
    }

    pub fn columns(&self)-> &[Column]{
        return &self.columns
    }

    pub fn column_index(&self,name:&str)->Result<usize>{
        for (index,column) in self.columns.iter().enumerate(){
            if column.name == name{
                return Ok(index);
            }
        }
        return Err(DbError::ColumnNotFound(name.to_string()))
    }

    pub fn column(&self,name: &str)->Result<&Column>{
        let index = self.column_index(name)?;
        Ok(&self.columns[index])
    }
    pub fn primary_key_index(&self)->usize{
        return self.primary_key_index
    }

    pub fn primary_key_column(&self)->&Column{
        &self.columns[self.primary_key_index]
    }
    pub fn validate_values(&self,values: &[Value])->Result<()>{
        if values.len() != self.columns.len(){
            return Err(DbError::InvalidRow(format!("expected {} values got {}",self.columns.len(),values.len())));
        }
        for (column, value) in self.columns.iter().zip(values.iter()){
            column.validate_value(value)?;
        }
        Ok(())
    }

    pub(crate) fn encode(&self) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        write_string(&mut bytes, &self.table_name)?;
        let count = u16::try_from(self.columns.len())
            .map_err(|_| DbError::InvalidSchema("too many columns".into()))?;
        bytes.extend_from_slice(&count.to_le_bytes());
        for column in &self.columns {
            write_string(&mut bytes, &column.name)?;
            bytes.push(match column.data_type {
                DataType::Integer => 0,
                DataType::Text => 1,
                DataType::Float => 2,
                DataType::Boolean => 3,
            });
            bytes.push(u8::from(column.nullable) | (u8::from(column.primary_key) << 1));
        }
        Ok(bytes)
    }

    pub(crate) fn decode(mut bytes: &[u8]) -> Result<Self> {
        let table_name = read_string(&mut bytes)?;
        let count = read_u16(&mut bytes)?;
        let mut columns = Vec::new();
        for _ in 0..count {
            let name = read_string(&mut bytes)?;
            let attributes = take(&mut bytes, 2)?;
            let data_type = match attributes[0] {
                0 => DataType::Integer,
                1 => DataType::Text,
                2 => DataType::Float,
                3 => DataType::Boolean,
                _ => return Err(DbError::CorruptData("invalid schema data type".into())),
            };
            let flags = attributes[1];
            if flags & !3 != 0 {
                return Err(DbError::CorruptData("invalid column flags".into()));
            }
            columns.push(Column::new(name, data_type, flags & 1 != 0, flags & 2 != 0));
        }
        if !bytes.is_empty() {
            return Err(DbError::CorruptData("trailing schema bytes".into()));
        }
        Self::new(table_name, columns)
            .map_err(|error| DbError::CorruptData(format!("invalid stored schema: {error}")))
    }
}

fn write_string(bytes: &mut Vec<u8>, value: &str) -> Result<()> {
    let length = u16::try_from(value.len())
        .map_err(|_| DbError::InvalidSchema("schema name is too long".into()))?;
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(value.as_bytes());
    Ok(())
}

fn take<'a>(bytes: &mut &'a [u8], length: usize) -> Result<&'a [u8]> {
    if length > bytes.len() {
        return Err(DbError::CorruptData("schema ended unexpectedly".into()));
    }
    let (value, remaining) = bytes.split_at(length);
    *bytes = remaining;
    Ok(value)
}

fn read_u16(bytes: &mut &[u8]) -> Result<u16> {
    let value = take(bytes, 2)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_string(bytes: &mut &[u8]) -> Result<String> {
    let length = usize::from(read_u16(bytes)?);
    let value = take(bytes, length)?;
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| DbError::CorruptData("invalid UTF-8 in schema".into()))
}
