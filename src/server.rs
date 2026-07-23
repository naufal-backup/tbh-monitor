use actix_web::{web, App, HttpServer, HttpResponse, middleware};
use actix_cors::Cors;
use serde_json::Value;
use std::sync::{Arc, RwLock};
use std::path::PathBuf;

pub struct AppState {
    pub save_data: Arc<RwLock<Option<Value>>>,
    pub _save_path: PathBuf,
}

pub async fn get_data(data: web::Data<AppState>) -> HttpResponse {
    let save_data = data.save_data.read().unwrap();
    
    match save_data.clone() {
        Some(json) => HttpResponse::Ok().json(json),
        None => HttpResponse::NotFound().json(serde_json::json!({
            "error": "No save data loaded"
        })),
    }
}

pub async fn get_inventory(data: web::Data<AppState>) -> HttpResponse {
    let save_data = data.save_data.read().unwrap();
    
    match save_data.clone() {
        Some(json) => {
            if let Some(player) = json.get("PlayerSaveData") {
                if let Some(value_str) = player.get("value").and_then(|v| v.as_str()) {
                    if let Ok(player_data) = serde_json::from_str::<Value>(value_str) {
                        if let Some(items) = player_data.get("inventoryItems") {
                            return HttpResponse::Ok().json(items);
                        }
                    }
                }
            }
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Inventory not found"
            }))
        }
        None => HttpResponse::NotFound().json(serde_json::json!({
            "error": "No save data loaded"
        })),
    }
}

pub async fn get_player(data: web::Data<AppState>) -> HttpResponse {
    let save_data = data.save_data.read().unwrap();
    
    match save_data.clone() {
        Some(json) => {
            if let Some(player) = json.get("PlayerSaveData") {
                if let Some(value_str) = player.get("value").and_then(|v| v.as_str()) {
                    if let Ok(player_data) = serde_json::from_str::<Value>(value_str) {
                        return HttpResponse::Ok().json(player_data);
                    }
                }
            }
            HttpResponse::NotFound().json(serde_json::json!({
                "error": "Player data not found"
            }))
        }
        None => HttpResponse::NotFound().json(serde_json::json!({
            "error": "No save data loaded"
        }))
    }
}

pub fn start_server(save_data: Arc<RwLock<Option<Value>>>, save_path: PathBuf, port: u16) {
    let data = web::Data::new(AppState {
        save_data,
        _save_path: save_path,
    });
    
    actix_web::rt::System::new()
        .block_on(async move {
            log::info!("Starting server on port {}", port);
            
            HttpServer::new(move || {
                let cors = Cors::permissive();
                
                App::new()
                    .wrap(cors)
                    .wrap(middleware::Logger::default())
                    .app_data(data.clone())
                    .route("/api/data", web::get().to(get_data))
                    .route("/api/inventory", web::get().to(get_inventory))
                    .route("/api/player", web::get().to(get_player))
            })
            .bind(format!("0.0.0.0:{}", port))
            .expect("Failed to start server")
            .run()
            .await
            .expect("Server error");
        });
}
