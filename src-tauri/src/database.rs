use rusqlite::{params, Connection, Result};

#[derive(Debug)]
struct Document {
    id: i32,
    name: String,
    data: Option<Vec<u8>>
}

fn main() -> Result<()>{
    let connection = Connection::open("database\atlas.db"); 
}