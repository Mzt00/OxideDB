use super::error::{DbError, Result};
use super::schema::Schema;

use super::value::{DataType,Value};

#[derive(Debug,Clone,PartialEq)]
pub struct Row{

    values: Vec<Value>,
}

impl Row{
    pub fn new(values: Vec<Value>)->Self{
        return Self{values}
    }

    pub fn values(&self)-> &[Value]{ //doesnt transfer ownership. read only
        &self.values
    }
    pub fn get(&self,index: usize)->Option<&Value>{
        return self.values.get(index)
    }

    pub fn set(&mut self, index: usize, value: Value)-> Result<()>{
       match self.values.get_mut(index) {
            Some(current_value) => {
                *current_value = value;

                return Ok(())
            }
            None => Err(DbError::InvalidRow(format!("column index {} is out of range",index))),
            }
    }
    pub fn validate(&self, schema: &Schema)->Result<()>{


        return schema.validate_values(&self.values)
    }
    pub fn primary_key<'a>(&'a self, schema: &Schema) -> Result<&'a Value> {
        let index = schema.primary_key_index();
         let value = self.values.get(index).ok_or_else(|| {
            DbError::InvalidRow("row does not contain its primary key value".to_string(),)})?;


        schema.primary_key_column().validate_value(value)?;
        Ok(value)
    }
    //Encoding format:
    //Null: 1 byte null marker
    //Integer:1 byte null + 8 bytes i64
    //boolean: 1 byte null+ 1 byte bool
    //float:1 byte null + 8 bytes f64
    //text:1 null+4 byte length +N bytes text
    pub fn encode(&self,schema:&Schema)->Result<Vec<u8>>{
        self.validate(schema)?;
        let mut bytes = Vec::new();
        for (column, value) in schema.columns().iter().zip(self.values.iter()){
            if value.is_null(){
                bytes.push(1);
                continue;
            }
            bytes.push(0);
            //uses little endian order
            match (column.data_type(),value){
                (DataType::Integer, Value::Integer(value))=> {
                    bytes.extend_from_slice(&value.to_le_bytes());

                }
                (DataType::Float, Value::Float(value)) => {
                    bytes.extend_from_slice(&value.to_le_bytes());
                }
                 (DataType::Boolean, Value::Boolean(value)) => {
                    bytes.push(if *value { 1 } else { 0 });
                }
                (DataType::Text, Value::Text(value)) => {

                    let text_bytes = value.as_bytes();



                    let length = u32::try_from(text_bytes.len()).map_err(|_| {
                        DbError::InvalidRow( 
                            "text value is too large".to_string(),
                        )
                    })?;
                    bytes.extend_from_slice(&length.to_le_bytes());
                    bytes.extend_from_slice(text_bytes);
                }
                 _ => {
                    return Err(DbError::InvalidRow( 

                        "row contains a value that does not match its schema" //kept here for future version
                            .to_string(),
                    ));
                }
            }
        }
        Ok(bytes)
    }
    pub fn decode(bytes: &[u8], schema: &Schema) -> Result<Self>{
        let mut values = Vec::with_capacity(schema.columns().len());
        let mut position = 0;
        for column in schema.columns(){

            let null_marker = read_bytes(bytes, &mut position, 1)?[0];
            match null_marker{

                1 =>{
                    values.push(Value::Null);
                    continue;
                }
                0 =>{}
                _ => {
                    return Err(DbError::CorruptData("Invalid Null marker".to_string(),));

                }

            }
            let value = match column.data_type(){
                DataType::Integer=>{
                    let data = read_bytes(bytes, &mut position, 8)?;

                    let array: [u8;8]= data.try_into().map_err(|_|{DbError::CorruptData("Invalid Integer data".to_string(),)})?;
                    Value::Integer(i64::from_le_bytes(array))
                }

                DataType::Float => {
                    let data = read_bytes(bytes, &mut position, 8)?;

                    let array: [u8; 8] = data.try_into().map_err(|_| {
                        DbError::CorruptData("Invalid float data".to_string(),)})?;
                    Value::Float(f64::from_le_bytes(array))
                }
                DataType::Boolean => {
                    let boolean_byte = read_bytes(
                        bytes,
                        &mut position,
                        1,
                    )?[0];

                    match boolean_byte {
                        0 => Value::Boolean(false),

                        1 => Value::Boolean(true),
                        _ => {
                            return Err(DbError::CorruptData("Invalid bool value".to_string(),));
                        }
                    }
                }
                DataType::Text=>{
                    let length_bytes = read_bytes(bytes,&mut position,4)?;
                    let length_array: [u8;4] = length_bytes.try_into().map_err(|_|{DbError::CorruptData("Invalid text length".to_string(),)})?;

                    let length = u32::from_le_bytes(length_array) as usize;

                    let text_bytes =  read_bytes(bytes,&mut position,length)?;

                     let text = std::str::from_utf8(text_bytes)
                        .map_err(|_| {DbError::CorruptData("Invalid text format".to_string(),)})?.to_string();
                    Value::Text(text)
                }
            };
            values.push(value);
        }
        if position!= bytes.len(){
            return Err(DbError::CorruptData("Corrupted data detected (trailing bytes)".to_string(),));
        }
        let row = Self {values};
        row.validate(schema)?;

        Ok(row)
    }
}
fn read_bytes<'a>(bytes: &'a [u8],position: &mut usize,length: usize,) -> Result<&'a [u8]> {
    let end = position.checked_add(length).ok_or_else(|| {
        DbError::CorruptData("Invalid byte length".to_string(),)})?;

    if end > bytes.len() {
        return Err(DbError::CorruptData("Row ended unexpectedly".to_string(),));
    }

    let data = &bytes[*position..end];

    *position = end;

    Ok(data)
}