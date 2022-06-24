async fn serve_files(
    config: &mut fpm::Config,
    path: std::path::PathBuf,
) -> actix_web::HttpResponse {
    let path = match path.to_str() {
        Some(s) => s,
        None => {
            println!("handle_ftd: Not able to convert path");
            return actix_web::HttpResponse::InternalServerError().body("".as_bytes());
        }
    };

    let f = match config.get_file_and_package_by_id(path).await {
        Ok(f) => f,
        Err(e) => {
            println!("new_path: {}, Error: {:?}", path, e);
            return actix_web::HttpResponse::InternalServerError().body(e.to_string());
        }
    };

    config.current_document = Some(f.get_id());
    return match f {
        fpm::File::Ftd(main_document) => {
            return match fpm::package_doc::read_ftd(config, &main_document, "/", false).await {
                Ok(r) => actix_web::HttpResponse::Ok().body(r),
                Err(e) => actix_web::HttpResponse::InternalServerError().body(e.to_string()),
            };
        }
        fpm::File::Image(image) => actix_web::HttpResponse::Ok()
            .content_type(
                infer::get(image.content.as_slice())
                    .map(|v| v.mime_type())
                    .unwrap_or(if image.id.ends_with(".svg") {
                        "image/svg+xml"
                    } else {
                        "image/jpeg"
                    }),
            )
            .body(image.content),
        _ => actix_web::HttpResponse::InternalServerError().body("".as_bytes()),
    };
}

/*async fn handle_dash(
    req: &actix_web::HttpRequest,
    config: &fpm::Config,
    path: std::path::PathBuf,
) -> actix_web::HttpResponse {
    let new_path = match path.to_str() {
        Some(s) => s.replace("-/", ""),
        None => {
            println!("handle_dash: Not able to convert path");
            return actix_web::HttpResponse::InternalServerError().body("".as_bytes());
        }
    };

    let file_path = if new_path.starts_with(&config.package.name) {
        std::path::PathBuf::new().join(
            new_path
                .strip_prefix(&(config.package.name.to_string() + "/"))
                .unwrap(),
        )
    } else {
        std::path::PathBuf::new().join(".packages").join(new_path)
    };

    server_static_file(req, file_path).await
}*/

async fn server_fpm_file(config: &fpm::Config) -> actix_web::HttpResponse {
    let response =
        match tokio::fs::read(config.get_root_for_package(&config.package).join("FPM.ftd")).await {
            Ok(res) => res,
            Err(e) => return actix_web::HttpResponse::NotFound().body(e.to_string()),
        };
    actix_web::HttpResponse::Ok()
        .content_type("application/octet-stream")
        .body(response)
}

async fn server_static_file(
    req: &actix_web::HttpRequest,
    file_path: std::path::PathBuf,
) -> actix_web::HttpResponse {
    if !file_path.exists() {
        return actix_web::HttpResponse::NotFound().body("".as_bytes());
    }

    match actix_files::NamedFile::open_async(file_path).await {
        Ok(r) => r.into_response(req),
        Err(_e) => actix_web::HttpResponse::NotFound().body("TODO".as_bytes()),
    }
}
async fn serve_static(req: actix_web::HttpRequest) -> actix_web::HttpResponse {
    let mut config = fpm::Config::read2(None, false).await.unwrap();
    let path: std::path::PathBuf = req.match_info().query("path").parse().unwrap();

    let favicon = std::path::PathBuf::new().join("favicon.ico");
    /*if path.starts_with("-/") {
        handle_dash(&req, &config, path).await
    } else*/
    if path.eq(&favicon) {
        server_static_file(&req, favicon).await
    } else if path.eq(&std::path::PathBuf::new().join("FPM.ftd")) {
        server_fpm_file(&config).await
    } else if path.eq(&std::path::PathBuf::new().join("")) {
        serve_files(&mut config, path.join("/")).await
    } else {
        serve_files(&mut config, path).await
    }
}

#[actix_web::main]
pub async fn serve2(bind_address: &str, port: Option<u16>) -> std::io::Result<()> {
    if cfg!(feature = "controller") {
        // fpm-controller base path and ec2 instance id (hardcoded for now)
        let fpm_controller: String = std::env::var("FPM_CONTROLLER")
            .unwrap_or_else(|_| "https://controller.fifthtry.com".to_string());
        let fpm_instance: String =
            std::env::var("FPM_INSTANCE_ID").expect("FPM_INSTANCE_ID is required");

        match crate::controller::resolve_dependencies(fpm_instance, fpm_controller).await {
            Ok(_) => println!("Dependencies resolved"),
            Err(e) => panic!("Error resolving dependencies using controller!!: {:?}", e),
        }
    }

    fn get_available_port(port: Option<u16>, bind_address: &str) -> Option<std::net::TcpListener> {
        let available_listener =
            |port: u16, bind_address: &str| std::net::TcpListener::bind((bind_address, port));

        if let Some(port) = port {
            return match available_listener(port, bind_address) {
                Ok(l) => Some(l),
                Err(_) => None,
            };
        }

        for x in 8000..9000 {
            match available_listener(x, bind_address) {
                Ok(l) => return Some(l),
                Err(_) => continue,
            }
        }
        None
    }

    let tcp_listener = match get_available_port(port, bind_address) {
        Some(listener) => listener,
        None => {
            eprintln!(
                "{}",
                port.map(|x| format!(
                    r#"provided port {} is not available, 
You can try without providing port, it will automatically pick unused port"#,
                    x
                ))
                .unwrap_or_else(|| {
                    "Tried picking port between port 8000 to 9000, not available -:(".to_string()
                })
            );
            return Ok(());
        }
    };

    let app = || {
        if cfg!(feature = "remote") {
            let json_cfg = actix_web::web::JsonConfig::default()
                .content_type(|mime| mime == mime_guess::mime::APPLICATION_JSON)
                .limit(9862416400);
            // .error_handler(|err, req| {
            //     actix_web::error::InternalError::from_response(
            //         err,
            //         actix_web::HttpResponse::Conflict().into(),
            //     )
            //     .into()
            // });

            actix_web::App::new()
                .app_data(json_cfg)
                .route("/-/sync/", actix_web::web::post().to(crate::apis::sync))
        } else {
            actix_web::App::new().route("/{path:.*}", actix_web::web::get().to(serve_static))
        }
    };

    println!("### Server Started ###");
    println!(
        "Go to: http://{}:{}",
        bind_address,
        tcp_listener.local_addr()?.port()
    );
    actix_web::HttpServer::new(app)
        .listen(tcp_listener)?
        .run()
        .await
}