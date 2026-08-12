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
}