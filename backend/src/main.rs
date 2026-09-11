use hydra::{get_auth, get_library, download_game_artifact, resolve_shortcut_app_id};

mod cloud_save;

mod hydra;

mod merge;

mod rules;

mod scanner;

mod wine;

fn optional_arg(value: Option<String>) -> Option<String> {

    value.filter(|v| !v.is_empty())

}

fn fail(message: &str) -> ! {

    println!("{}", serde_json::json!({ "ok": false, "error": message }));

    std::process::exit(1);

}

fn arg(index: usize, name: &str) -> String {

    std::env::args().nth(index).unwrap_or_else(|| fail(&format!("no {name} given")))

}

fn read_auth_from_stdin() -> String {

    let mut buffer = String::new();

    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut buffer).is_err() {

        fail("failed to read auth from stdin");

    }

    buffer.trim().to_string()

}

#[tokio::main]

async fn main() {

    let command = std::env::args().nth(1).unwrap_or_else(|| fail("no command given"));

    match command.as_str() {

        "get-auth" => {

            let auth = get_auth();

            println!("{}", auth);

        }

        "get-library" => {

            let library = get_library();

            println!("{}", library);

        }

        "download-game-artifact" => {

            let object_id = arg(2, "object id");

            let download_url = arg(3, "download url");

            let object_key = arg(4, "object key");

            let home_dir = arg(5, "home dir");

            let wine_prefix = arg(6, "wine prefix");

            let artifact_wine_prefix = std::env::args().nth(7);

            if let Err(err) = download_game_artifact(&object_id, "steam", &download_url, &object_key, &home_dir, Some(&wine_prefix), artifact_wine_prefix).await {

                println!("{}", serde_json::json!({ "ok": false, "error": format!("{err:#}") }));

                std::process::exit(1);

            }

            println!("{}", serde_json::json!({ "ok": true }));

        }

        "resolve-shortcut" => {

            let app_id = arg(2, "app id");

            match app_id.parse::<u32>() {

                Ok(id) => {

                    let result: Option<hydra::ShortcutResolution> = resolve_shortcut_app_id(id);

                    println!("{}", serde_json::to_string(&result).unwrap());

                }

                Err(_) => {

                    println!("{}", serde_json::json!({ "ok": false, "error": "invalid app id" }));

                    std::process::exit(1);

                }

            }

        }

        "sync-cloud-save" => {

            let auth_json = read_auth_from_stdin();

            let object_id = arg(2, "object id");

            let wine_prefix = optional_arg(std::env::args().nth(3));

            let force = std::env::args().nth(4).as_deref() == Some("force");

            let resolutions = optional_arg(std::env::args().nth(5))
                .map(|json| match serde_json::from_str(&json) {
                    Ok(resolutions) => resolutions,
                    Err(_) => fail("invalid resolutions json"),
                });

            let shop = optional_arg(std::env::args().nth(6)).unwrap_or_else(|| "steam".to_string());

            match cloud_save::sync_cloud_save(&auth_json, &object_id, &shop, wine_prefix.as_deref(), force, resolutions).await {

                Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),

                Err(err) => {

                    println!("{}", serde_json::json!({ "ok": false, "error": format!("{err:#}") }));

                    std::process::exit(1);

                }

            }

        }

        "restore-cloud-save" => {

            let auth_json = read_auth_from_stdin();

            let object_id = arg(2, "object id");

            let wine_prefix = optional_arg(std::env::args().nth(3));

            let shop = optional_arg(std::env::args().nth(4)).unwrap_or_else(|| "steam".to_string());

            match cloud_save::restore_cloud_save(&auth_json, &object_id, &shop, wine_prefix.as_deref()).await {

                Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),

                Err(err) => {

                    println!("{}", serde_json::json!({ "ok": false, "error": format!("{err:#}") }));

                    std::process::exit(1);

                }

            }

        }

        "check-cloud-save-status" => {

            let auth_json = read_auth_from_stdin();

            let object_id = arg(2, "object id");

            let wine_prefix = optional_arg(std::env::args().nth(3));

            let shop = optional_arg(std::env::args().nth(4)).unwrap_or_else(|| "steam".to_string());

            match cloud_save::check_cloud_save_status(&auth_json, &object_id, &shop, wine_prefix.as_deref()).await {

                Ok(result) => println!("{}", serde_json::to_string(&result).unwrap()),

                Err(err) => {

                    println!("{}", serde_json::json!({ "ok": false, "error": format!("{err:#}") }));

                    std::process::exit(1);

                }

            }

        }

        _ => {

            fail("invalid command");

        }

    }

}

