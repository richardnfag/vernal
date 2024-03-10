use heed::flags::Flags;
use heed::{types::*, Env};
use heed::{Database, EnvOpenOptions};
use std::fs;
use std::io::Error;
use std::path::Path;

use chrono::{SecondsFormat::Micros, Utc};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[derive(Debug, Clone, Copy)]
pub struct Transaction {
    applied_at: [u8; 27],
    value: u32,
    kind: char,
    description: [u8; 10],
    client_id: u8,
    balance: i32,
    limit: i32,
}

#[derive(Debug)]
pub struct Balance {
    total: i32,
    date: [u8; 27],
    limit: i32,
}

#[derive(Debug)]
pub struct Statement {
    balance: Balance,
    last_transactions: Vec<Transaction>,
}

fn limit_by_client_id(client_id: u8) -> i32 {
    match client_id {
        1 => 100_000,
        2 => 80_000,
        3 => 1_000_000,
        4 => 10_000_000,
        5 => 500_000,
        _ => 0,
    }
}

impl Transaction {
    pub fn new(
        applied_at: &str,
        value: u32,
        kind: char,
        description: &str,
        client_id: u8,
    ) -> Transaction {
        let limit = limit_by_client_id(client_id);

        let mut description_bytes: [u8; 10] = [
            b'\0', b'\0', b'\0', b'\0', b'\0', b'\0', b'\0', b'\0', b'\0', b'\0',
        ];

        let mut applied_at_bytes: [u8; 27] = [b'\0'; 27];
        applied_at_bytes[..27].clone_from_slice(applied_at.as_bytes());

        let description = description.as_bytes();

        for i in 0..description.len() {
            description_bytes[i] = description[i];
        }

        Transaction {
            applied_at: applied_at_bytes,
            value,
            kind,
            description: description_bytes,
            client_id,
            balance: 0,
            limit,
        }
    }

    pub fn encode(&self) -> [u8; 20] {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(&self.value.to_be_bytes());
        bytes.push(self.kind as u8);
        bytes.extend_from_slice(&self.aligned_description());
        bytes.extend_from_slice(&self.client_id.to_be_bytes());
        bytes.extend_from_slice(&self.balance.to_be_bytes());

        let mut result: [u8; 20] = [0; 20];

        for i in 0..bytes.len() {
            result[i] = bytes[i];
        }

        result
    }

    pub fn decode(bytes: [u8; 20], applied_at: &str) -> Transaction {
        let applied_at: [u8; 27] = applied_at.as_bytes().try_into().unwrap();
        let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let kind = char::from(bytes[4] as u8);
        let description = [
            bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11], bytes[12],
            bytes[13], bytes[14],
        ];
        let client_id = u8::from_be_bytes([bytes[15]]);
        let balance = i32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);

        let limit = limit_by_client_id(client_id);

        Transaction {
            applied_at,
            value,
            kind,
            description,
            client_id,
            balance,
            limit,
        }
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"valor\": {}, \"tipo\": \"{}\", \"descricao\": \"{}\", \"realizada_em\": \"{}\"}}",
            self.value,
            self.kind,
            (unsafe { String::from_utf8_unchecked(self.description.to_vec()) })
                .trim_end_matches('\0'),
            unsafe { String::from_utf8_unchecked(self.applied_at.to_vec()) },
        )
    }

    fn aligned_description(&self) -> [u8; 10] {
        let mut aligned_description: [u8; 10] = [0; 10];

        for i in 0..10 {
            aligned_description[i] = self.description[i];
        }

        aligned_description
    }
}

impl Statement {
    pub fn new(client_id: u8, total: i32, last_transactions: Vec<Transaction>) -> Statement {
        let limit = limit_by_client_id(client_id);

        let mut date_bytes: [u8; 27] = [b'\0'; 27];
        date_bytes[..27].clone_from_slice(get_current_time().as_bytes());

        Statement {
            balance: Balance {
                total,
                date: date_bytes,
                limit,
            },
            last_transactions,
        }
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"saldo\": {{\"total\": {}, \"data_extrato\": \"{}\", \"limite\": {}}}, \"ultimas_transacoes\": [{}]}}",
            self.balance.total,
            unsafe { String::from_utf8_unchecked(self.balance.date.to_vec()) },
            self.balance.limit,
            self.last_transactions
                .iter()
                .rev()
                .map(|t| t.to_json())
                .collect::<Vec<String>>()
                .join(", ")
        )
    }
}

async fn store_transaction(
    db_path: &str,
    new_transaction: Transaction,
) -> Result<(i32, i32), Error> {
    let (env, db) = create_database(new_transaction.client_id, db_path).await;

    let mut wtxn = env.write_txn().unwrap();

    let mut new_transaction = new_transaction;

    let last_transaction_encoded = db.last(&wtxn).unwrap();

    let last_transaction = match last_transaction_encoded {
        Some((key, value)) => Transaction::decode(value, key),
        None => Transaction::new(
            "0000-00-00T00:00:00.000000Z",
            0,
            'c',
            "Initial",
            new_transaction.client_id,
        ),
    };

    let current_date = get_current_time();

    new_transaction.applied_at = current_date.as_bytes().try_into().unwrap();

    new_transaction.balance = match new_transaction.kind {
        'c' => last_transaction.balance + new_transaction.value as i32,
        'd' => {
            if (last_transaction.balance - new_transaction.value as i32).abs()
                <= last_transaction.limit
            {
                last_transaction.balance - new_transaction.value as i32
            } else {
                wtxn.abort().unwrap();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    "Limite de saldo excedido",
                ));
            }
        }

        _ => {
            wtxn.abort().unwrap();
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Invalid kind of transaction",
            ));
        }
    };

    db.put(&mut wtxn, current_date.as_str(), &new_transaction.encode())
        .unwrap();

    wtxn.commit().unwrap();

    env.force_sync().unwrap();

    Ok((new_transaction.balance, new_transaction.limit))
}

async fn get_statement(db_path: &str, client_id: u8) -> Result<Statement, Error> {
    let (env, db) = create_database(client_id, db_path).await;

    let rtxn = env.read_txn().unwrap();

    let mut last_transactions = Vec::new();

    let transactions = db.iter(&rtxn).unwrap();

    for result in transactions.into_iter() {
        match result {
            Ok((key, value)) => {
                let t = Transaction::decode(value, &key);
                last_transactions.push(t);
            }
            Err(_) => {}
        }
    }

    if last_transactions.len() == 0 {
        return Ok(Statement::new(client_id, 0, Vec::new()));
    }

    let last_transaction = last_transactions.last().unwrap();
    let total = last_transaction.balance;

    Ok(Statement::new(client_id, total, last_transactions))
}

fn get_current_time() -> String {
    Utc::now().to_rfc3339_opts(Micros, true)
}

macro_rules! write_not_found_response {
    ($stream:expr) => {
        $stream
            .write_all("HTTP/1.1 404 Not Found\r\n\r\n".as_bytes())
            .await
            .unwrap()
    };
}

macro_rules! write_unprocessable_entity_response {
    ($stream:expr) => {
        $stream
            .write_all(b"HTTP/1.1 422 Unprocessable Entity\r\n\r\n")
            .await
            .unwrap()
    };
}

macro_rules! write_ok_response {
    ($stream:expr, $contents:expr) => {
        $stream
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {length}\r\n\r\n{contents}",
                    length = $contents.len(),
                    contents = $contents
                )
                .as_bytes(),
            )
            .await.unwrap()
    };
}

async fn create_database(
    client_id: u8,
    db_path: &str,
) -> (Env, Database<Str, OwnedType<[u8; 20]>>) {
    let env_path = Path::new(db_path).join(format!("client{}.mdb", client_id));

    fs::create_dir_all(&env_path).unwrap();

    let env = unsafe {
        let mut env_builder = EnvOpenOptions::new();

        env_builder.flag(Flags::MdbNoTls);
        env_builder.flag(Flags::MdbNoMetaSync);
        env_builder.flag(Flags::MdbNoSync);
        env_builder.flag(Flags::MdbNoLock);

        env_builder
            .max_dbs(2)
            .map_size(1024 * 100_000)
            .max_readers(10)
            .open(env_path)
            .unwrap()
    };

    let db = env
        .create_database(Some(format!("client{}", client_id).as_str()))
        .unwrap();

    (env, db)
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let port = std::env::var("HTTP_SERVER_PORT")
        .unwrap_or_default()
        .parse::<u16>()
        .unwrap_or(9999);
    let listener = TcpListener::bind(("0.0.0.0", port)).await?;

    println!("Server running on port: {}", port);

    loop {
        let (mut stream, _) = listener.accept().await?;

        tokio::spawn(async move {
            let mut buffer = [0; 256];

            let db_path = std::env::var("VERNAL_DB_PATH").unwrap_or("/app/data".to_string());

            loop {
                match stream.read(&mut buffer).await {
                    Ok(0) => return,
                    Ok(_n) => {
                        let header_end = buffer
                            .windows(4)
                            .position(|window| window == [b'\r', b'\n', b'\r', b'\n'])
                            .expect("Invalid request");

                        let header = &buffer[..header_end];
                        let lines: Vec<&[u8]> =
                            header.split(|&c| c == b'\r' || c == b'\n').collect();

                        let uri = std::str::from_utf8(lines[0]).expect("Invalid request");

                        let mut parts = uri.split('/');
                        let (verb, clients, client_id, operation) =
                            (parts.next(), parts.next(), parts.next(), parts.next());

                        match (verb, clients, client_id, operation) {
                            (
                                Some("POST "),
                                Some("clientes"),
                                Some(id),
                                Some("transacoes HTTP"),
                            ) => match id {
                                "1" | "2" | "3" | "4" | "5" => {
                                    let id = id.parse::<u8>().unwrap();
                                    let body = &buffer[header_end + 4..];

                                    let values_result = parse_body_to_transaction_values(body);

                                    let (valor, tipo, descricao) = match values_result {
                                        Ok((valor, tipo, descricao)) => (valor, tipo, descricao),
                                        Err(_) => {
                                            write_unprocessable_entity_response!(stream);
                                            return;
                                        }
                                    };

                                    let result = store_transaction(
                                        &db_path,
                                        Transaction::new(
                                            &get_current_time(),
                                            valor,
                                            tipo,
                                            descricao.as_str(),
                                            id,
                                        ),
                                    )
                                    .await;

                                    match result {
                                        Ok((total, limit)) => {
                                            let response = format!(
                                                "{{\"limite\": {}, \"saldo\": {}}}",
                                                limit, total
                                            );

                                            write_ok_response!(stream, response);
                                        }
                                        Err(_) => {
                                            write_unprocessable_entity_response!(stream);
                                            return;
                                        }
                                    };
                                }
                                _ => {
                                    write_not_found_response!(stream);
                                    return;
                                }
                            },
                            (Some("GET "), Some("clientes"), Some(id), Some("extrato HTTP")) => {
                                match id {
                                    "1" | "2" | "3" | "4" | "5" => {
                                        let result =
                                            get_statement(&db_path, id.parse::<u8>().unwrap())
                                                .await;

                                        match result {
                                            Ok(statement) => {
                                                let response = statement.to_json();
                                                write_ok_response!(stream, response);
                                            }
                                            Err(_) => {
                                                write_unprocessable_entity_response!(stream);
                                                return;
                                            }
                                        };
                                    }
                                    _ => {
                                        write_not_found_response!(stream);
                                        return;
                                    }
                                };
                            }
                            _ => {
                                write_not_found_response!(stream);
                            }
                        };
                    }
                    Err(_) => {
                        write_not_found_response!(stream);
                        return;
                    }
                }
            }
        });
    }
}

fn parse_body_to_transaction_values(body: &[u8]) -> Result<(u32, char, String), Error> {
    let body_str = std::str::from_utf8(body).expect("Invalid UTF-8 in body");
    let (mut valor, mut tipo, mut descricao) = (0_u32, String::new(), String::new());

    for pair in body_str.split(',') {
        let mut tokens = pair.split(':');
        let key = tokens.next().expect("Invalid JSON");
        let value = tokens.next().expect("Invalid JSON");
        let key_trimmed = key.trim_matches('{').trim().trim_matches('"');
        let value_trimmed = value
            .trim_matches(' ')
            .trim_matches('\0')
            .trim_matches('}')
            .trim()
            .trim_matches('"');

        match key_trimmed {
            "valor" => valor = value_trimmed.parse::<u32>().unwrap_or(0),
            "tipo" => tipo = value_trimmed.to_string(),
            "descricao" => descricao = value_trimmed.to_string(),
            _ => {}
        }
    }

    if descricao.len() > 10 || descricao == "null" || descricao == "" || valor == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Invalid JSON",
        ));
    }

    let tipo = match tipo.as_str() {
        "c" => 'c',
        "d" => 'd',
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Invalid kind of transaction",
            ));
        }
    };

    Ok((valor, tipo, descricao))
}
