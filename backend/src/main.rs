use ludusavi::{get_backup_preview, check_if_ludusavi_binary_exists};
use hydra::{get_auth, get_library, upload_save_game, download_game_artifact};

mod ludusavi;
mod hydra;
mod wine;

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
        "backup-and-upload" => {
            let object_id = std::env::args().nth(2).expect("no object id given");
            let wine_prefix = std::env::args().nth(3).expect("no wine prefix given");
            let access_token = std::env::args().nth(4).expect("no access token given");
            let label = std::env::args().nth(5).expect("no label given");

            upload_save_game(&object_id, "steam", Some(&wine_prefix), &access_token, &label).await.unwrap();
        }
        "download-game-artifact" => {
            let object_id = std::env::args().nth(2).expect("no object id given");
            let download_url = std::env::args().nth(3).expect("no download url given");
            let object_key = std::env::args().nth(4).expect("no object key given");
            let home_dir = std::env::args().nth(5).expect("no home dir given");
            let wine_prefix = std::env::args().nth(6).expect("no wine prefix given");
            let artifact_wine_prefix = std::env::args().nth(7).expect("no artifact wine prefix given");

            download_game_artifact(&object_id, "steam", &download_url, &object_key, &home_dir, Some(&wine_prefix), Some(&artifact_wine_prefix)).await.unwrap();
        }
        "check-if-ludusavi-binary-exists" => {
            let exists = check_if_ludusavi_binary_exists();
            println!("{}", exists);
        }
        _ => {
            println!("Invalid command");
        }
    }
}
