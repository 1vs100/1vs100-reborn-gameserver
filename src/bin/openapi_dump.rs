//! Prints an OpenAPI document: the admin API's by default, consumed by `frontend`'s
//! `api:generate`, or with `status` the public status API's, consumed by `site`.
fn main() {
    let document = match std::env::args().nth(1).as_deref() {
        None => onevs100_gameserver::admin::openapi_json(),
        Some("status") => onevs100_gameserver::status::openapi_json(),
        Some(other) => {
            eprintln!("unknown API {other:?}: expected no argument or `status`");
            std::process::exit(2);
        }
    };
    println!("{document}");
}
