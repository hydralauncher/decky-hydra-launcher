use ludusavi::{get_backup_preview, check_if_ludusavi_binary_exists};
use hydra::{get_auth, get_library, download_game_artifact};

mod cloud_save;
mod ludusavi;
mod hydra;
mod wine;

fn optional_arg(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// Auth JSON is read from stdin so tokens never appear in the process
/// argument list (`/proc/<pid>/cmdline` is world-readable).
fn read_auth_from_stdin() -> String {
    let mut buffer = String::new();
    std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer)
        .expect("failed to read auth from stdin");
    buffer.trim().to_string()
}

#[tokio::main]
async fn main() {
    let command = std::env::args().nth(1).expect("no command given");
    match command.as_str() {
        "get-auth" => {
            let auth = get_auth();
            println!("{}", auth);
        }
        "get-library" => {
            let library = get_library();
            println!("{}", library);
        }
        "get-backup-preview" => {
            let object_id = std::env::args().nth(2).expect("no object id given");
            let wine_prefix = std::env::args().nth(3).expect("no wine prefix given");
            let preview = get_backup_preview(&object_id, Some(&wine_prefix)).await.unwrap();
            println!("{}", preview);
        }
        "download-game-artifact" => {
            let object_id = std::env::args().nth(2).expect("no object id given");
            let download_url = std::env::args().nth(3).expect("no download url given");
            let object_key = std::env::args().nth(4).expect("no object key given");
            let home_dir = std::env::args().nth(5).expect("no home dir given");
            let wine_prefix = std::env::args().nth(6).expect("no wine prefix given");
            let artifact_wine_prefix = std::env::args().nth(7);

            download_game_artifact(&object_id, "steam", &download_url, &object_key, &home_dir, Some(&wine_prefix), artifact_wine_prefix).await.unwrap();
        }
        "check-if-ludusavi-binary-exists" => {
            let exists = check_if_ludusavi_binary_exists();
            println!("{}", exists);
        }
        "sync-cloud-save" => {
            let auth_json = read_auth_from_stdin();
            let object_id = std::env::args().nth(2).expect("no object id given");
            let wine_prefix = optional_arg(std::env::args().nth(3));

            match cloud_save::sync_cloud_save(&auth_json, &object_id, "steam", wine_prefix.as_deref()).await {
                Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),
                Err(err) => {
                    println!("{}", serde_json::json!({ "ok": false, "error": format!("{err:#}") }));
                    std::process::exit(1);
                }
            }
        }
        "restore-cloud-save" => {
            let auth_json = read_auth_from_stdin();
            let object_id = std::env::args().nth(2).expect("no object id given");
            let wine_prefix = optional_arg(std::env::args().nth(3));

            match cloud_save::restore_cloud_save(&auth_json, &object_id, "steam", wine_prefix.as_deref()).await {
                Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),
                Err(err) => {
                    println!("{}", serde_json::json!({ "ok": false, "error": format!("{err:#}") }));
                    std::process::exit(1);
                }
            }
        }
        "check-cloud-save-status" => {
            let auth_json = read_auth_from_stdin();
            let object_id = std::env::args().nth(2).expect("no object id given");

            match cloud_save::check_cloud_save_status(&auth_json, &object_id, "steam").await {
                Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),
                Err(err) => {
                    println!("{}", serde_json::json!({ "ok": false, "error": format!("{err:#}") }));
                    std::process::exit(1);
                }
            }
        }
        _ => {
            println!("Invalid command");
        }
    }
}
