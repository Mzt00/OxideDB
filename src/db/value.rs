use std::fmt;

#[derive(Debug, Clone,Copy, PartialEq, Eq)]
pub enum DataType{
    Integer,
    Text,
    Float,
    Boolean,
}

impl fmt::Display for DataType{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result{
        match self{
            DataType::Integer=>write!(f,"INTEGER"),
            DataType::Text=>write!(f,"TEXT"),
            DataType::Float=>write!(f,"FLOAT"),
            DataType::Boolean=>write!(f,"BOOLEAN"),

        }
    }
}

#[derive(Debug, Clone,PartialEq)]
pub enum Value{
    Null,
    Integer(i64),
    Text(String),
    Float(f64),
    Boolean(bool),
}

impl Value{

    pub fn data_type(&self)-> Option<DataType>{
        match self{
            Value::Null=> None,
            Value::Integer(_)=>Some(DataType::Integer),
            Value::Text(_)=>Some(DataType::Text),
            Value::Float(_)=>Some(DataType::Float),
            Value::Boolean(_)=>Some(DataType::Boolean),
        }
    }
    pub fn is_null(&self)->bool{
        matches!(self,Value::Null)
    }

    pub fn match_type(&self, expected: DataType)->bool{
        match self{
            Value::Null=> true, //Null to be checked in schema layer
            Value::Integer(_)=> expected==DataType::Integer,
            Value::Text(_)=> expected==DataType::Text,
            Value::Float(_)=> expected==DataType::Float,
            Value::Boolean(_)=> expected==DataType::Boolean,
        }

    }
}

impl fmt::Display for Value{
     fn fmt(&self, f: &mut fmt::Formatter<'_>)-> fmt::Result{
        match self{
            Value::Null=>write!(f,"NULL"),
            Value::Integer(value)=>write!(f,"{}",value),
            Value::Text(value)=>write!(f,"{}",value),
            Value::Float(value)=>write!(f,"{}",value),
            Value::Boolean(value)=>write!(f,"{}",value),

        }
    }
}