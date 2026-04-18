use anyhow::{Context as _, Result, bail};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE, URL_SAFE_NO_PAD},
};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use serde_yaml_ng::Mapping;
use std::collections::HashMap;
use tauri::Url;

#[derive(Debug, Clone)]
pub struct NormalizedSubscription {
    pub yaml: String,
    pub suggested_name: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct TranslatedConfig {
    proxies: Vec<JsonMap<String, JsonValue>>,
    proxy_providers: JsonMap<String, JsonValue>,
    proxy_groups: Vec<JsonMap<String, JsonValue>>,
    rules: Vec<String>,
    rule_providers: JsonMap<String, JsonValue>,
    warnings: Vec<String>,
}

pub fn looks_like_inline_source(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }

    if is_supported_yaml(trimmed) || looks_like_uri_collection(trimmed) || looks_like_external_profile(trimmed) {
        return true;
    }

    let decoded = decode_base64_or_original(trimmed);
    decoded != trimmed
        && (is_supported_yaml(&decoded) || looks_like_uri_collection(&decoded) || looks_like_external_profile(&decoded))
}

pub fn normalize_inline_source(input: &str) -> Result<NormalizedSubscription> {
    normalize_subscription_text(input)
}

pub fn normalize_subscription_text(input: &str) -> Result<NormalizedSubscription> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("subscription content is empty");
    }

    if is_supported_yaml(trimmed) {
        return Ok(NormalizedSubscription {
            yaml: trimmed.to_owned(),
            suggested_name: None,
        });
    }

    if let Some(parsed) = parse_uri_collection(trimmed)? {
        return Ok(parsed);
    }

    let decoded = decode_base64_or_original(trimmed);
    if decoded != trimmed {
        if is_supported_yaml(&decoded) {
            return Ok(NormalizedSubscription {
                yaml: decoded.trim().to_owned(),
                suggested_name: None,
            });
        }

        if let Some(parsed) = parse_uri_collection(&decoded)? {
            return Ok(parsed);
        }
    }

    if let Some(parsed) = normalize_external_profile(trimmed)? {
        return Ok(parsed);
    }

    if decoded != trimmed {
        if let Some(parsed) = normalize_external_profile(&decoded)? {
            return Ok(parsed);
        }
    }

    bail!("unsupported subscription format or share link");
}

fn is_supported_yaml(input: &str) -> bool {
    serde_yaml_ng::from_str::<Mapping>(input)
        .ok()
        .is_some_and(|yaml| yaml.contains_key("proxies") || yaml.contains_key("proxy-providers"))
}

fn looks_like_uri_collection(input: &str) -> bool {
    let lines = input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_comment_line(line))
        .collect::<Vec<_>>();

    !lines.is_empty() && lines.iter().all(|line| looks_like_supported_uri(line))
}

fn parse_uri_collection(input: &str) -> Result<Option<NormalizedSubscription>> {
    let lines = input
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !is_comment_line(line))
        .collect::<Vec<_>>();

    if lines.is_empty() {
        return Ok(None);
    }

    let support_flags = lines
        .iter()
        .map(|line| looks_like_supported_uri(line))
        .collect::<Vec<_>>();
    if support_flags.iter().all(|supported| !supported) {
        return Ok(None);
    }

    if let Some((idx, _)) = support_flags.iter().enumerate().find(|(_, supported)| !**supported) {
        bail!("unsupported share link in line {}", idx + 1);
    }

    let mut proxies = Vec::with_capacity(lines.len());
    for line in lines {
        proxies.push(parse_uri(line)?);
    }

    let suggested_name = match proxies.len() {
        0 => None,
        1 => proxies[0]
            .get("name")
            .and_then(JsonValue::as_str)
            .map(std::borrow::ToOwned::to_owned),
        len => proxies[0]
            .get("name")
            .and_then(JsonValue::as_str)
            .map(|name| format!("{name} +{}", len - 1)),
    };

    let yaml = generate_mihomo_yaml(proxies)?;
    Ok(Some(NormalizedSubscription { yaml, suggested_name }))
}

fn generate_mihomo_yaml(proxies: Vec<JsonMap<String, JsonValue>>) -> Result<String> {
    generate_translated_yaml(TranslatedConfig {
        proxies,
        ..TranslatedConfig::default()
    })
}

fn generate_translated_yaml(mut config: TranslatedConfig) -> Result<String> {
    let mut proxies = config.proxies;
    dedupe_proxy_names(&mut proxies);

    let proxy_names = proxies
        .iter()
        .filter_map(|proxy| {
            proxy
                .get("name")
                .and_then(JsonValue::as_str)
                .map(std::borrow::ToOwned::to_owned)
        })
        .collect::<Vec<_>>();

    let provider_names = config.proxy_providers.keys().cloned().collect::<Vec<_>>();

    if proxy_names.is_empty() && provider_names.is_empty() {
        bail!("no valid proxy names generated");
    }

    let default_target = if config.proxy_groups.is_empty() {
        if !proxy_names.is_empty() {
            let mut group_proxies = proxy_names.clone();
            group_proxies.push("DIRECT".to_owned());
            config.proxy_groups.push(
                json_object_to_map(json!({
                    "name": "PROXY",
                    "type": "select",
                    "proxies": group_proxies,
                }))
                .context("failed to build default proxy group")?,
            );
        } else {
            config.proxy_groups.push(
                json_object_to_map(json!({
                    "name": "PROXY",
                    "type": "select",
                    "use": provider_names,
                }))
                .context("failed to build default provider-backed proxy group")?,
            );
        }
        "PROXY".to_owned()
    } else {
        config
            .proxy_groups
            .first()
            .and_then(|group| group.get("name"))
            .and_then(JsonValue::as_str)
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_else(|| proxy_names[0].clone())
    };

    if config.rules.is_empty() {
        config.rules.push(format!("MATCH,{default_target}"));
    }

    let mut root = JsonMap::new();
    root.insert(
        "proxies".into(),
        JsonValue::Array(proxies.into_iter().map(JsonValue::Object).collect()),
    );
    if !config.proxy_providers.is_empty() {
        root.insert("proxy-providers".into(), JsonValue::Object(config.proxy_providers));
    }
    root.insert(
        "proxy-groups".into(),
        JsonValue::Array(config.proxy_groups.into_iter().map(JsonValue::Object).collect()),
    );
    if !config.rule_providers.is_empty() {
        root.insert("rule-providers".into(), JsonValue::Object(config.rule_providers));
    }
    root.insert(
        "rules".into(),
        JsonValue::Array(config.rules.into_iter().map(JsonValue::String).collect()),
    );

    let yaml =
        serde_yaml_ng::to_string(&JsonValue::Object(root)).context("failed to serialize generated Mihomo profile")?;

    if config.warnings.is_empty() {
        return Ok(yaml);
    }

    let mut prefix = String::from("# Generated by Clash Verge Rev\n# Translation notes:");
    for warning in dedupe_warnings(config.warnings) {
        prefix.push_str("\n# - ");
        prefix.push_str(&warning);
    }
    prefix.push_str("\n\n");
    prefix.push_str(&yaml);
    Ok(prefix)
}

fn dedupe_proxy_names(proxies: &mut [JsonMap<String, JsonValue>]) {
    let mut seen: HashMap<String, usize> = HashMap::new();
    for proxy in proxies {
        let Some(name) = proxy
            .get("name")
            .and_then(JsonValue::as_str)
            .map(std::borrow::ToOwned::to_owned)
        else {
            continue;
        };

        let entry = seen.entry(name.clone()).or_insert(0);
        *entry += 1;
        if *entry > 1 {
            proxy.insert("name".into(), JsonValue::String(format!("{name} {:02}", *entry)));
        }
    }
}

fn is_comment_line(line: &str) -> bool {
    matches!(line.chars().next(), Some('#' | ';')) || line.starts_with("//")
}

fn looks_like_supported_uri(line: &str) -> bool {
    let Some(scheme) = uri_scheme(line) else {
        return false;
    };

    match scheme.as_str() {
        "http" | "https" => looks_like_http_proxy_uri(line),
        "ss" | "ssr" | "vmess" | "vless" | "trojan" | "ssh" | "snell" | "anytls" | "hysteria" | "hy" | "hysteria2"
        | "hy2" | "tuic" | "socks" | "socks5" | "wireguard" | "wg" | "mieru" | "masque" | "sudoku" => true,
        _ => false,
    }
}

fn uri_scheme(input: &str) -> Option<String> {
    let trimmed = input.trim();
    let (scheme, rest) = trimmed.split_once("://")?;
    if scheme.is_empty() || rest.is_empty() {
        return None;
    }
    Some(scheme.to_ascii_lowercase())
}

fn looks_like_http_proxy_uri(input: &str) -> bool {
    let Ok(url) = Url::parse(input) else {
        return false;
    };

    if !matches!(url.scheme(), "http" | "https") {
        return false;
    }

    let has_userinfo = !url.username().is_empty() || url.password().is_some();
    let has_fragment = url.fragment().is_some();
    let has_port = url.port().is_some();
    let is_root_path = matches!(url.path(), "" | "/");
    let has_non_proxy_query = url
        .query()
        .is_some_and(|query| query.contains('=') || query.contains('&'));

    has_fragment || (has_userinfo && is_root_path) || (has_port && is_root_path && !has_non_proxy_query)
}

fn parse_uri(line: &str) -> Result<JsonMap<String, JsonValue>> {
    match uri_scheme(line).as_deref() {
        Some("ss") => parse_ss(line),
        Some("ssr") => parse_ssr(line),
        Some("vmess") => parse_vmess(line),
        Some("vless") => parse_vless(line),
        Some("trojan") => parse_trojan(line),
        Some("ssh") => parse_ssh(line),
        Some("snell") => parse_snell(line),
        Some("anytls") => parse_anytls(line),
        Some("mieru") => parse_mieru(line),
        Some("masque") => parse_masque(line),
        Some("sudoku") => parse_sudoku(line),
        Some("hysteria") | Some("hy") => parse_hysteria(line),
        Some("hysteria2") | Some("hy2") => parse_hysteria2(line),
        Some("tuic") => parse_tuic(line),
        Some("http") | Some("https") => parse_http(line),
        Some("socks") | Some("socks5") => parse_socks(line),
        Some("wireguard") | Some("wg") => parse_wireguard(line),
        Some(scheme) => bail!("unsupported uri scheme: {scheme}"),
        None => bail!("invalid uri: missing scheme"),
    }
}

fn parse_ss(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let after_scheme = strip_uri_scheme(line, &["ss"])?;
    let (without_hash, hash_raw) = split_once(after_scheme, '#');
    let name = decode_and_trim(hash_raw).unwrap_or_default();

    let (main_raw, query_raw) = split_once(without_hash, '?');
    let query_params = parse_query_string(query_raw);

    let main = if main_raw.contains('@') {
        main_raw.to_owned()
    } else {
        decode_base64_or_original(main_raw)
    };

    let at_idx = main.rfind('@').context("invalid ss uri: missing '@'")?;
    let userinfo = decode_base64_or_original(&main[..at_idx]);
    let server_and_port = main[at_idx + 1..]
        .split('/')
        .next()
        .context("invalid ss uri: missing server")?;
    let port_idx = server_and_port.rfind(':').context("invalid ss uri: missing port")?;
    let server = &server_and_port[..port_idx];
    let port = parse_required_port(&server_and_port[port_idx + 1..], "invalid ss uri: invalid port")?;
    let (cipher, password_raw) = split_once(&userinfo, ':');
    let password = password_raw.context("invalid ss uri: missing password")?;

    let mut proxy = base_proxy_map(
        "ss",
        if name.is_empty() {
            format!("SS {server}:{port}")
        } else {
            name
        },
        server,
        port,
    );
    proxy.insert("cipher".into(), JsonValue::String(normalize_cipher(cipher)));
    proxy.insert("password".into(), JsonValue::String(password.to_owned()));

    if let Some(plugin_param) = query_params.get("plugin") {
        let plugin_parts = plugin_param.split(';').collect::<Vec<_>>();
        if let Some(plugin_name) = plugin_parts.first().copied() {
            match plugin_name {
                "obfs-local" | "simple-obfs" => {
                    proxy.insert("plugin".into(), JsonValue::String("obfs".into()));
                    let mut opts = JsonMap::new();
                    for raw in plugin_parts.into_iter().skip(1) {
                        let (key, value) = split_once(raw, '=');
                        match key {
                            "obfs" if !value.unwrap_or_default().is_empty() => {
                                opts.insert("mode".into(), JsonValue::String(value.unwrap_or_default().to_owned()));
                            }
                            "obfs-host" if !value.unwrap_or_default().is_empty() => {
                                opts.insert("host".into(), JsonValue::String(value.unwrap_or_default().to_owned()));
                            }
                            _ => {}
                        }
                    }
                    if !opts.is_empty() {
                        proxy.insert("plugin-opts".into(), JsonValue::Object(opts));
                    }
                }
                "v2ray-plugin" => {
                    proxy.insert("plugin".into(), JsonValue::String("v2ray-plugin".into()));
                    let mut opts = JsonMap::new();
                    opts.insert("mode".into(), JsonValue::String("websocket".into()));
                    for raw in plugin_parts.into_iter().skip(1) {
                        let (key, value) = split_once(raw, '=');
                        match key {
                            "tls" => {
                                opts.insert("tls".into(), JsonValue::Bool(true));
                            }
                            "host" | "obfs-host" if !value.unwrap_or_default().is_empty() => {
                                opts.insert("host".into(), JsonValue::String(value.unwrap_or_default().to_owned()));
                            }
                            "path" if !value.unwrap_or_default().is_empty() => {
                                opts.insert("path".into(), JsonValue::String(value.unwrap_or_default().to_owned()));
                            }
                            _ => {}
                        }
                    }
                    if !opts.is_empty() {
                        proxy.insert("plugin-opts".into(), JsonValue::Object(opts));
                    }
                }
                _ => {}
            }
        }
    }

    if let Some(v2ray_plugin) = query_params.get("v2ray-plugin") {
        if !proxy.contains_key("plugin") {
            let decoded = decode_base64_or_original(v2ray_plugin);
            if let Ok(value) = serde_json::from_str::<JsonValue>(&decoded) {
                proxy.insert("plugin".into(), JsonValue::String("v2ray-plugin".into()));
                proxy.insert("plugin-opts".into(), value);
            }
        }
    }

    if boolish_key_present(&query_params, "uot") {
        proxy.insert("udp-over-tcp".into(), JsonValue::Bool(true));
    }
    if boolish_key_present(&query_params, "tfo") {
        proxy.insert("tfo".into(), JsonValue::Bool(true));
    }

    Ok(proxy)
}

fn parse_ssr(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let decoded = decode_base64_or_original(strip_uri_scheme(line, &["ssr"])?);
    let split_idx = decoded
        .find(":origin")
        .or_else(|| decoded.find(":auth_"))
        .context("invalid ssr uri")?;
    let server_and_port = &decoded[..split_idx];
    let port_idx = server_and_port.rfind(':').context("invalid ssr uri: missing port")?;
    let server = &server_and_port[..port_idx];
    let port = parse_required_port(&server_and_port[port_idx + 1..], "invalid ssr uri: invalid port")?;

    let params_section = &decoded[split_idx + 1..];
    let basic = params_section
        .split("/?")
        .next()
        .unwrap_or_default()
        .split(':')
        .collect::<Vec<_>>();
    if basic.len() < 4 {
        bail!("invalid ssr uri");
    }

    let other_params = parse_query_string(decoded.split("/?").nth(1));
    let mut proxy = base_proxy_map("ssr", "SSR".into(), server, port);
    proxy.insert("protocol".into(), JsonValue::String(basic[0].to_owned()));
    proxy.insert("cipher".into(), JsonValue::String(normalize_cipher(basic[1])));
    proxy.insert("obfs".into(), JsonValue::String(basic[2].to_owned()));
    proxy.insert(
        "password".into(),
        JsonValue::String(decode_base64_or_original(basic[3])),
    );

    if let Some(remarks) = other_params
        .get("remarks")
        .map(|value| decode_base64_or_original(value).trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        proxy.insert("name".into(), JsonValue::String(remarks));
    } else {
        proxy.insert("name".into(), JsonValue::String(server.to_owned()));
    }

    if let Some(protocol_param) = other_params
        .get("protoparam")
        .map(|value| value.chars().filter(|ch| !ch.is_whitespace()).collect::<String>())
        .map(|value| decode_base64_or_original(&value))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("protocol-param".into(), JsonValue::String(protocol_param));
    }

    if let Some(obfs_param) = other_params
        .get("obfsparam")
        .map(|value| value.chars().filter(|ch| !ch.is_whitespace()).collect::<String>())
        .map(|value| decode_base64_or_original(&value))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("obfs-param".into(), JsonValue::String(obfs_param));
    }

    Ok(proxy)
}

fn parse_vmess(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let raw = strip_uri_scheme(line, &["vmess"])?;
    let content = decode_base64_or_original(raw);

    if content.contains("=vmess") {
        return parse_vmess_quantumult(&content);
    }

    let params = match serde_json::from_str::<serde_json::Map<String, JsonValue>>(&content) {
        Ok(json) => json
            .into_iter()
            .map(|(key, value)| (key, json_value_to_string(value)))
            .collect::<HashMap<_, _>>(),
        Err(_) => parse_vmess_shadowrocket_params(raw),
    };

    let server = params
        .get("add")
        .map(String::as_str)
        .context("invalid vmess uri: missing server")?;
    let port = parse_required_port_opt(
        params.get("port").map(String::as_str),
        "invalid vmess uri: invalid port",
    )?;
    let name = params
        .get("ps")
        .or_else(|| params.get("remarks"))
        .or_else(|| params.get("remark"))
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("VMess {server}:{port}"));

    let mut proxy = base_proxy_map("vmess", name, server, port);
    proxy.insert(
        "cipher".into(),
        JsonValue::String(
            params
                .get("scy")
                .map(String::as_str)
                .map(normalize_cipher)
                .unwrap_or_else(|| "auto".to_owned()),
        ),
    );
    proxy.insert(
        "uuid".into(),
        JsonValue::String(params.get("id").cloned().context("invalid vmess uri: missing uuid")?),
    );

    if let Some(aid) = params
        .get("aid")
        .or_else(|| params.get("alterId"))
        .and_then(|value| value.parse::<u64>().ok())
    {
        proxy.insert("alterId".into(), JsonValue::Number(aid.into()));
    }

    let tls_enabled = params
        .get("tls")
        .is_some_and(|value| matches!(value.as_str(), "tls" | "1" | "true" | "TRUE" | "True"));
    if tls_enabled {
        proxy.insert("tls".into(), JsonValue::Bool(true));
    }

    if let Some(skip_verify) = params.get("verify_cert").and_then(|value| parse_bool(value)) {
        proxy.insert("skip-cert-verify".into(), JsonValue::Bool(!skip_verify));
    }

    if tls_enabled {
        if let Some(sni) = params.get("sni").filter(|value| !value.is_empty()) {
            proxy.insert("servername".into(), JsonValue::String(sni.clone()));
        }
    }

    let mut httpupgrade = false;
    let network = match params
        .get("net")
        .or_else(|| params.get("obfs"))
        .or_else(|| params.get("type"))
        .map(String::as_str)
    {
        Some("ws") | Some("websocket") => Some("ws"),
        Some("http") => Some("http"),
        Some("grpc") => Some("grpc"),
        Some("httpupgrade") => {
            httpupgrade = true;
            Some("ws")
        }
        Some("h2") => Some("h2"),
        _ => None,
    };

    if let Some(network) = network {
        proxy.insert("network".into(), JsonValue::String(network.to_owned()));
    }

    let transport_host = params
        .get("host")
        .or_else(|| params.get("obfsParam"))
        .map(String::as_str)
        .filter(|value| !value.is_empty());
    let transport_path = params.get("path").map(String::as_str).filter(|value| !value.is_empty());

    match network {
        Some("grpc") => {
            if let Some(service_name) = transport_path {
                proxy.insert("grpc-opts".into(), json!({ "grpc-service-name": service_name }));
            }
        }
        Some("h2") => {
            let mut opts = JsonMap::new();
            if let Some(host) = transport_host {
                opts.insert("host".into(), JsonValue::String(host.to_owned()));
            }
            if let Some(path) = transport_path {
                opts.insert("path".into(), JsonValue::String(path.to_owned()));
            }
            if !opts.is_empty() {
                proxy.insert("h2-opts".into(), JsonValue::Object(opts));
            }
        }
        Some("http") => {
            let mut opts = JsonMap::new();
            let mut paths = vec![transport_path.unwrap_or("/").to_owned()];
            paths.retain(|path| !path.is_empty());
            if paths.is_empty() {
                paths.push("/".into());
            }
            opts.insert(
                "path".into(),
                JsonValue::Array(paths.into_iter().map(JsonValue::String).collect()),
            );
            if let Some(host) = transport_host {
                opts.insert("headers".into(), json!({ "Host": [host] }));
            }
            proxy.insert("http-opts".into(), JsonValue::Object(opts));
        }
        Some("ws") => {
            let mut opts = JsonMap::new();
            if let Some(path) = transport_path {
                opts.insert("path".into(), JsonValue::String(path.to_owned()));
            }
            if let Some(host) = transport_host {
                let host_header = transport_host_json_host(host).unwrap_or_else(|| host.to_owned());
                opts.insert("headers".into(), json!({ "Host": host_header }));
            }
            if httpupgrade {
                opts.insert("v2ray-http-upgrade".into(), JsonValue::Bool(true));
                opts.insert("v2ray-http-upgrade-fast-open".into(), JsonValue::Bool(true));
            }
            if !opts.is_empty() {
                proxy.insert("ws-opts".into(), JsonValue::Object(opts));
            }
        }
        _ => {}
    }

    if tls_enabled && !proxy.contains_key("servername") {
        if let Some(host) = transport_host.map(transport_host_json_host).flatten() {
            proxy.insert("servername".into(), JsonValue::String(host));
        } else if let Some(host) = transport_host {
            proxy.insert("servername".into(), JsonValue::String(host.to_owned()));
        }
    }

    Ok(proxy)
}

fn parse_vmess_quantumult(content: &str) -> Result<JsonMap<String, JsonValue>> {
    let partitions = content.split(',').map(str::trim).collect::<Vec<_>>();
    if partitions.len() < 5 {
        bail!("invalid vmess quantumult uri");
    }

    let mut params = HashMap::new();
    for part in &partitions {
        if let Some((key, value)) = part.split_once('=') {
            params.insert(key.trim().to_owned(), value.trim().to_owned());
        }
    }

    let mut proxy = base_proxy_map(
        "vmess",
        partitions[0].split('=').next().unwrap_or_default().trim().to_owned(),
        partitions[1],
        parse_required_port(partitions[2], "invalid vmess uri: invalid port")?,
    );
    proxy.insert("cipher".into(), JsonValue::String(normalize_cipher(partitions[3])));
    proxy.insert(
        "uuid".into(),
        JsonValue::String(partitions[4].trim_matches('"').to_owned()),
    );

    if matches!(params.get("obfs").map(String::as_str), Some("wss")) {
        proxy.insert("tls".into(), JsonValue::Bool(true));
    }
    if let Some(udp_relay) = params.get("udp-relay").and_then(|value| parse_bool(value)) {
        proxy.insert("udp".into(), JsonValue::Bool(udp_relay));
    }
    if let Some(fast_open) = params.get("fast-open").and_then(|value| parse_bool(value)) {
        proxy.insert("tfo".into(), JsonValue::Bool(fast_open));
    }
    if params.contains_key("tls-verification") {
        let tls_ok = params
            .get("tls-verification")
            .and_then(|value| parse_bool(value))
            .unwrap_or(false);
        proxy.insert("skip-cert-verify".into(), JsonValue::Bool(!tls_ok));
    }

    if matches!(params.get("obfs").map(String::as_str), Some("ws" | "wss")) {
        proxy.insert("network".into(), JsonValue::String("ws".into()));
        let path = params
            .get("obfs-path")
            .map(|value| value.trim_matches('"').to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "/".into());
        let host = params
            .get("obfs-header")
            .and_then(|value| value.split("Host:").nth(1))
            .map(str::trim)
            .unwrap_or_default()
            .to_owned();
        proxy.insert(
            "ws-opts".into(),
            json!({
                "path": path,
                "headers": { "Host": host },
            }),
        );
    }

    Ok(proxy)
}

fn parse_vmess_shadowrocket_params(raw: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();
    let Some((base64_line, query)) = raw.split_once('?') else {
        return params;
    };

    let content = decode_base64_or_original(base64_line.trim_end_matches('/'));
    for addon in query.split('&') {
        if addon.is_empty() {
            continue;
        }
        let (key, value) = split_once(addon, '=');
        if key.trim().is_empty() {
            continue;
        }
        params.insert(key.trim().to_owned(), percent_decode(value.unwrap_or_default()));
    }

    if let Some(at_idx) = content.rfind('@') {
        let userinfo = &content[..at_idx];
        let server_part = &content[at_idx + 1..];
        if let Some(port_idx) = server_part.rfind(':') {
            let (cipher, uuid) = split_once(userinfo, ':');
            params.insert("scy".into(), cipher.to_owned());
            params.insert("id".into(), uuid.unwrap_or_default().to_owned());
            params.insert("add".into(), server_part[..port_idx].to_owned());
            params.insert("port".into(), server_part[port_idx + 1..].to_owned());
        }
    }

    params
}

fn parse_vless(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let mut effective_line = line.to_owned();
    let parsed = match Url::parse(&effective_line) {
        Ok(url) => url,
        Err(_) => {
            let after_scheme = strip_uri_scheme(line, &["vless"])?;
            let (prefix, suffix) = after_scheme.split_once('?').context("invalid vless uri")?;
            effective_line = format!("vless://{}?{}", decode_base64_or_original(prefix), suffix);
            Url::parse(&effective_line).context("invalid vless uri")?
        }
    };

    let server = parsed.host_str().context("invalid vless uri: missing host")?;
    let port = parsed.port().context("invalid vless uri: missing port")?;
    let uuid = percent_decode(parsed.username());
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| {
        params
            .get("remarks")
            .or_else(|| params.get("remark"))
            .cloned()
            .unwrap_or_else(|| format!("VLESS {server}:{port}"))
    });

    let mut proxy = base_proxy_map("vless", name, server, port);
    proxy.insert("uuid".into(), JsonValue::String(uuid));

    let mut tls_enabled = params.get("security").is_some_and(|value| value != "none");
    if !tls_enabled
        && params
            .get("tls")
            .is_some_and(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "True"))
    {
        tls_enabled = true;
    }
    if tls_enabled {
        proxy.insert("tls".into(), JsonValue::Bool(true));
    }

    if let Some(servername) = params
        .get("sni")
        .or_else(|| params.get("peer"))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("servername".into(), JsonValue::String(servername.clone()));
    }

    if let Some(flow) = params.get("flow").filter(|value| is_valid_vless_flow(value)) {
        proxy.insert("flow".into(), JsonValue::String(flow.clone()));
    }

    if let Some(client_fp) = params
        .get("fp")
        .or_else(|| params.get("client-fingerprint"))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("client-fingerprint".into(), JsonValue::String(client_fp.clone()));
    }

    if let Some(alpn) = params.get("alpn") {
        let values = split_csv(alpn);
        if !values.is_empty() {
            proxy.insert(
                "alpn".into(),
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
            );
        }
    }

    if params.contains_key("allowInsecure") {
        proxy.insert(
            "skip-cert-verify".into(),
            JsonValue::Bool(boolish_value(params.get("allowInsecure"))),
        );
    }

    if matches!(params.get("security").map(String::as_str), Some("reality")) {
        let mut reality_opts = JsonMap::new();
        if let Some(public_key) = params.get("pbk").filter(|value| !value.is_empty()) {
            reality_opts.insert("public-key".into(), JsonValue::String(public_key.clone()));
        }
        if let Some(short_id) = params.get("sid").filter(|value| !value.is_empty()) {
            reality_opts.insert("short-id".into(), JsonValue::String(short_id.clone()));
        }
        if !reality_opts.is_empty() {
            proxy.insert("reality-opts".into(), JsonValue::Object(reality_opts));
        }
    }

    let mut httpupgrade = false;
    let network = match params.get("headerType").map(String::as_str) {
        Some("http") => Some("http"),
        _ => match params.get("type").map(String::as_str) {
            Some("websocket") | Some("ws") => Some("ws"),
            Some("grpc") => Some("grpc"),
            Some("h2") => Some("h2"),
            Some("http") => Some("http"),
            Some("httpupgrade") => {
                httpupgrade = true;
                Some("ws")
            }
            Some("tcp") | None => Some("tcp"),
            _ => Some("tcp"),
        },
    };

    if let Some(network) = network.filter(|network| *network != "tcp") {
        proxy.insert("network".into(), JsonValue::String(network.to_owned()));
    }

    let host = params
        .get("host")
        .or_else(|| params.get("obfsParam"))
        .filter(|value| !value.is_empty());
    let path = params.get("path").filter(|value| !value.is_empty());

    match network {
        Some("grpc") => {
            if let Some(service_name) = path {
                proxy.insert("grpc-opts".into(), json!({ "grpc-service-name": service_name }));
            }
        }
        Some("h2") => {
            let mut opts = JsonMap::new();
            if let Some(host) = host {
                opts.insert("host".into(), JsonValue::String(host.clone()));
            }
            if let Some(path) = path {
                opts.insert("path".into(), JsonValue::String(path.clone()));
            }
            if !opts.is_empty() {
                proxy.insert("h2-opts".into(), JsonValue::Object(opts));
            }
        }
        Some("http") => {
            let mut opts = JsonMap::new();
            if let Some(path) = path {
                opts.insert("path".into(), JsonValue::Array(vec![JsonValue::String(path.clone())]));
            }
            if let Some(host) = host {
                opts.insert("headers".into(), json!({ "Host": [host] }));
            }
            if !opts.is_empty() {
                proxy.insert("http-opts".into(), JsonValue::Object(opts));
            }
        }
        Some("ws") => {
            let mut opts = JsonMap::new();
            if let Some(host) = host {
                if params.contains_key("obfsParam") {
                    if let Ok(headers) = serde_json::from_str::<JsonValue>(host) {
                        opts.insert("headers".into(), headers);
                    } else {
                        opts.insert("headers".into(), json!({ "Host": host }));
                    }
                } else {
                    opts.insert("headers".into(), json!({ "Host": host }));
                }
            }
            if let Some(path) = path {
                opts.insert("path".into(), JsonValue::String(path.clone()));
            }
            if httpupgrade {
                opts.insert("v2ray-http-upgrade".into(), JsonValue::Bool(true));
                opts.insert("v2ray-http-upgrade-fast-open".into(), JsonValue::Bool(true));
            }
            if !opts.is_empty() {
                proxy.insert("ws-opts".into(), JsonValue::Object(opts));
            }
        }
        _ => {}
    }

    if tls_enabled && !proxy.contains_key("servername") {
        if let Some(host) = host {
            proxy.insert(
                "servername".into(),
                JsonValue::String(transport_host_json_host(host).unwrap_or_else(|| host.clone())),
            );
        }
    }

    Ok(proxy)
}

fn parse_trojan(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid trojan uri")?;
    let server = parsed.host_str().context("invalid trojan uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let password = percent_decode(parsed.username());
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("Trojan {server}:{port}"));

    let mut proxy = base_proxy_map("trojan", name, server, port);
    proxy.insert("password".into(), JsonValue::String(password));

    if let Some(network) = params
        .get("type")
        .map(String::as_str)
        .filter(|value| matches!(*value, "ws" | "grpc" | "h2" | "tcp"))
    {
        if network != "tcp" {
            proxy.insert("network".into(), JsonValue::String(network.to_owned()));
        }
    }

    if let Some(alpn) = params.get("alpn") {
        let values = split_csv(alpn);
        if !values.is_empty() {
            proxy.insert(
                "alpn".into(),
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
            );
        }
    }
    if let Some(sni) = params
        .get("sni")
        .or_else(|| params.get("peer"))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("sni".into(), JsonValue::String(sni.clone()));
    }
    if params.contains_key("skip-cert-verify") {
        proxy.insert(
            "skip-cert-verify".into(),
            JsonValue::Bool(boolish_value(params.get("skip-cert-verify"))),
        );
    }
    if let Some(fingerprint) = params
        .get("fingerprint")
        .or_else(|| params.get("fp"))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("fingerprint".into(), JsonValue::String(fingerprint.clone()));
    }
    if let Some(encryption) = params.get("encryption") {
        let parts = encryption.split(';').collect::<Vec<_>>();
        if parts.len() == 3 {
            proxy.insert(
                "ss-opts".into(),
                json!({
                    "enabled": true,
                    "method": parts[1],
                    "password": parts[2],
                }),
            );
        }
    }
    if let Some(client_fp) = params.get("client-fingerprint").filter(|value| !value.is_empty()) {
        proxy.insert("client-fingerprint".into(), JsonValue::String(client_fp.clone()));
    }

    let host = params.get("host").filter(|value| !value.is_empty());
    let path = params.get("path").filter(|value| !value.is_empty());
    match params.get("type").map(String::as_str) {
        Some("ws") => {
            let mut opts = JsonMap::new();
            if let Some(host) = host {
                opts.insert("headers".into(), json!({ "Host": host }));
            }
            if let Some(path) = path {
                opts.insert("path".into(), JsonValue::String(path.clone()));
            }
            if !opts.is_empty() {
                proxy.insert("ws-opts".into(), JsonValue::Object(opts));
            }
        }
        Some("grpc") => {
            if let Some(service_name) = path {
                proxy.insert("grpc-opts".into(), json!({ "grpc-service-name": service_name }));
            }
        }
        _ => {}
    }

    Ok(proxy)
}

fn parse_ssh(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid ssh uri")?;
    let server = parsed.host_str().context("invalid ssh uri: missing host")?;
    let port = parsed.port().unwrap_or(22);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("SSH {server}:{port}"));

    let mut proxy = base_proxy_map("ssh", name, server, port);
    if !parsed.username().is_empty() {
        proxy.insert("username".into(), JsonValue::String(percent_decode(parsed.username())));
    }
    if let Some(password) = parsed.password() {
        proxy.insert("password".into(), JsonValue::String(percent_decode(password)));
    }
    if let Some(private_key) = params.get("private-key").filter(|value| !value.is_empty()) {
        proxy.insert("private-key".into(), JsonValue::String(private_key.clone()));
    }
    if let Some(passphrase) = params.get("private-key-passphrase").filter(|value| !value.is_empty()) {
        proxy.insert("private-key-passphrase".into(), JsonValue::String(passphrase.clone()));
    }
    if let Some(host_key) = params.get("host-key").filter(|value| !value.is_empty()) {
        proxy.insert("host-key".into(), JsonValue::String(host_key.clone()));
    }
    if let Some(algorithms) = params.get("host-key-algorithms").filter(|value| !value.is_empty()) {
        proxy.insert("host-key-algorithms".into(), JsonValue::String(algorithms.clone()));
    }

    Ok(proxy)
}

fn parse_snell(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid snell uri")?;
    let server = parsed.host_str().context("invalid snell uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("Snell {server}:{port}"));

    let mut proxy = base_proxy_map("snell", name, server, port);
    if !parsed.username().is_empty() {
        proxy.insert("psk".into(), JsonValue::String(percent_decode(parsed.username())));
    }
    if let Some(password) = parsed.password() {
        proxy.insert("psk".into(), JsonValue::String(percent_decode(password)));
    }
    if let Some(version) = params.get("version").and_then(|value| parse_integer(Some(value))) {
        proxy.insert("version".into(), JsonValue::Number(version.into()));
    }
    if params.contains_key("udp") {
        proxy.insert("udp".into(), JsonValue::Bool(boolish_value(params.get("udp"))));
    }

    Ok(proxy)
}

fn parse_mieru(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid mieru uri")?;
    let server = parsed.host_str().context("invalid mieru uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("Mieru {server}:{port}"));

    let mut proxy = base_proxy_map("mieru", name, server, port);
    if !parsed.username().is_empty() {
        proxy.insert("username".into(), JsonValue::String(percent_decode(parsed.username())));
    }
    if let Some(password) = parsed.password() {
        proxy.insert("password".into(), JsonValue::String(percent_decode(password)));
    }

    if let Some(username) = params.get("username").filter(|value| !value.is_empty()) {
        proxy.insert("username".into(), JsonValue::String(username.clone()));
    }
    if let Some(password) = params.get("password").filter(|value| !value.is_empty()) {
        proxy.insert("password".into(), JsonValue::String(password.clone()));
    }
    if let Some(port_range) = params.get("port-range").filter(|value| !value.is_empty()) {
        proxy.insert("port-range".into(), JsonValue::String(port_range.clone()));
    }
    if let Some(transport) = normalize_upper_enum(params.get("transport"), &["TCP", "UDP"]) {
        proxy.insert("transport".into(), JsonValue::String(transport));
    }
    if let Some(multiplexing) = normalize_upper_enum(
        params.get("multiplexing"),
        &[
            "MULTIPLEXING_OFF",
            "MULTIPLEXING_LOW",
            "MULTIPLEXING_MIDDLE",
            "MULTIPLEXING_HIGH",
        ],
    ) {
        proxy.insert("multiplexing".into(), JsonValue::String(multiplexing));
    }
    if let Some(handshake_mode) = params.get("handshake-mode").filter(|value| !value.is_empty()) {
        proxy.insert("handshake-mode".into(), JsonValue::String(handshake_mode.clone()));
    }
    if params.contains_key("udp") {
        proxy.insert("udp".into(), JsonValue::Bool(boolish_value(params.get("udp"))));
    }

    Ok(proxy)
}

fn parse_masque(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid masque uri")?;
    let server = parsed.host_str().context("invalid masque uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("Masque {server}:{port}"));

    let mut proxy = base_proxy_map("masque", name, server, port);
    if !parsed.username().is_empty() {
        proxy.insert(
            "private-key".into(),
            JsonValue::String(percent_decode(parsed.username())),
        );
    } else if let Some(password) = parsed.password() {
        proxy.insert("private-key".into(), JsonValue::String(percent_decode(password)));
    }

    if let Some(private_key) = params.get("private-key").filter(|value| !value.is_empty()) {
        proxy.insert("private-key".into(), JsonValue::String(private_key.clone()));
    }
    if let Some(public_key) = params.get("public-key").filter(|value| !value.is_empty()) {
        proxy.insert("public-key".into(), JsonValue::String(public_key.clone()));
    }
    if let Some(ip) = params.get("ip").filter(|value| !value.is_empty()) {
        proxy.insert("ip".into(), JsonValue::String(ip.clone()));
    }
    if let Some(ipv6) = params.get("ipv6").filter(|value| !value.is_empty()) {
        proxy.insert("ipv6".into(), JsonValue::String(ipv6.clone()));
    }
    if let Some(mtu) = params.get("mtu").and_then(|value| parse_integer(Some(value))) {
        proxy.insert("mtu".into(), JsonValue::Number(mtu.into()));
    }
    if params.contains_key("udp") {
        proxy.insert("udp".into(), JsonValue::Bool(boolish_value(params.get("udp"))));
    }
    if params.contains_key("remote-dns-resolve") {
        proxy.insert(
            "remote-dns-resolve".into(),
            JsonValue::Bool(boolish_value(params.get("remote-dns-resolve"))),
        );
    }
    if let Some(dns) = params.get("dns") {
        let values = split_csv(dns);
        if !values.is_empty() {
            proxy.insert(
                "dns".into(),
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
            );
        }
    }

    Ok(proxy)
}

fn parse_sudoku(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid sudoku uri")?;
    let server = parsed.host_str().context("invalid sudoku uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("Sudoku {server}:{port}"));

    let mut proxy = base_proxy_map("sudoku", name, server, port);
    if !parsed.username().is_empty() {
        proxy.insert("key".into(), JsonValue::String(percent_decode(parsed.username())));
    } else if let Some(password) = parsed.password() {
        proxy.insert("key".into(), JsonValue::String(percent_decode(password)));
    }

    if let Some(key) = params.get("key").filter(|value| !value.is_empty()) {
        proxy.insert("key".into(), JsonValue::String(key.clone()));
    }
    if let Some(aead_method) =
        normalize_lower_enum(params.get("aead-method"), &["chacha20-poly1305", "aes-128-gcm", "none"])
    {
        proxy.insert("aead-method".into(), JsonValue::String(aead_method));
    }
    if let Some(padding_min) = params.get("padding-min").and_then(|value| parse_integer(Some(value))) {
        proxy.insert("padding-min".into(), JsonValue::Number(padding_min.into()));
    }
    if let Some(padding_max) = params.get("padding-max").and_then(|value| parse_integer(Some(value))) {
        proxy.insert("padding-max".into(), JsonValue::Number(padding_max.into()));
    }
    if let Some(table_type) = normalize_lower_enum(params.get("table-type"), &["prefer_ascii", "prefer_entropy"]) {
        proxy.insert("table-type".into(), JsonValue::String(table_type));
    }
    if params.contains_key("enable-pure-downlink") {
        proxy.insert(
            "enable-pure-downlink".into(),
            JsonValue::Bool(boolish_value(params.get("enable-pure-downlink"))),
        );
    }
    if params.contains_key("http-mask") {
        proxy.insert(
            "http-mask".into(),
            JsonValue::Bool(boolish_value(params.get("http-mask"))),
        );
    }
    if let Some(http_mask_mode) =
        normalize_lower_enum(params.get("http-mask-mode"), &["legacy", "stream", "poll", "auto"])
    {
        proxy.insert("http-mask-mode".into(), JsonValue::String(http_mask_mode));
    }
    if params.contains_key("http-mask-tls") {
        proxy.insert(
            "http-mask-tls".into(),
            JsonValue::Bool(boolish_value(params.get("http-mask-tls"))),
        );
    }
    if let Some(http_mask_host) = params.get("http-mask-host").filter(|value| !value.is_empty()) {
        proxy.insert("http-mask-host".into(), JsonValue::String(http_mask_host.clone()));
    }
    if let Some(http_mask_strategy) =
        normalize_lower_enum(params.get("http-mask-strategy"), &["random", "post", "websocket"])
    {
        proxy.insert("http-mask-strategy".into(), JsonValue::String(http_mask_strategy));
    }
    if let Some(custom_table) = params.get("custom-table").filter(|value| !value.is_empty()) {
        proxy.insert("custom-table".into(), JsonValue::String(custom_table.clone()));
    }
    if let Some(custom_tables) = params.get("custom-tables") {
        let values = split_csv(custom_tables);
        if !values.is_empty() {
            proxy.insert(
                "custom-tables".into(),
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
            );
        }
    }

    Ok(proxy)
}

fn parse_anytls(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid anytls uri")?;
    let server = parsed.host_str().context("invalid anytls uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let auth = percent_decode(parsed.username());
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("AnyTLS {server}:{port}"));

    let mut proxy = base_proxy_map("anytls", name, server, port);
    if !auth.is_empty() {
        let password = auth
            .split_once(':')
            .map(|(_, password)| password.to_owned())
            .unwrap_or(auth);
        proxy.insert("password".into(), JsonValue::String(password));
    }

    if let Some(sni) = params.get("sni").filter(|value| !value.is_empty()) {
        proxy.insert("sni".into(), JsonValue::String(sni.clone()));
    }
    if let Some(alpn) = params.get("alpn") {
        let values = split_csv(alpn);
        if !values.is_empty() {
            proxy.insert(
                "alpn".into(),
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
            );
        }
    }
    if let Some(fingerprint) = params
        .get("fingerprint")
        .or_else(|| params.get("hpkp"))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("fingerprint".into(), JsonValue::String(fingerprint.clone()));
    }
    if let Some(client_fp) = params
        .get("client-fingerprint")
        .or_else(|| params.get("fp"))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("client-fingerprint".into(), JsonValue::String(client_fp.clone()));
    }
    if params.contains_key("skip-cert-verify") || params.contains_key("insecure") {
        let flag = params.get("skip-cert-verify").or_else(|| params.get("insecure"));
        proxy.insert("skip-cert-verify".into(), JsonValue::Bool(boolish_value(flag)));
    }
    if params.contains_key("udp") {
        proxy.insert("udp".into(), JsonValue::Bool(boolish_value(params.get("udp"))));
    }
    if let Some(number) = params
        .get("idle-session-check-interval")
        .and_then(|value| parse_integer(Some(value)))
    {
        proxy.insert("idle-session-check-interval".into(), JsonValue::Number(number.into()));
    }
    if let Some(number) = params
        .get("idle-session-timeout")
        .and_then(|value| parse_integer(Some(value)))
    {
        proxy.insert("idle-session-timeout".into(), JsonValue::Number(number.into()));
    }
    if let Some(number) = params
        .get("min-idle-session")
        .and_then(|value| parse_integer(Some(value)))
    {
        proxy.insert("min-idle-session".into(), JsonValue::Number(number.into()));
    }

    Ok(proxy)
}

fn parse_hysteria(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid hysteria uri")?;
    let server = parsed.host_str().context("invalid hysteria uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("Hysteria {server}:{port}"));

    let mut proxy = base_proxy_map("hysteria", name, server, port);
    for (key, value) in params {
        match key.as_str() {
            "alpn" => {
                let values = split_csv(&value);
                if !values.is_empty() {
                    proxy.insert(
                        "alpn".into(),
                        JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
                    );
                }
            }
            "insecure" => {
                proxy.insert("skip-cert-verify".into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            "auth" | "auth-str" if !value.is_empty() => {
                proxy.insert("auth-str".into(), JsonValue::String(value));
            }
            "mport" | "ports" if !value.is_empty() => {
                proxy.insert("ports".into(), JsonValue::String(value));
            }
            "obfsParam" | "obfs-param" | "obfs" => {
                proxy.insert("obfs".into(), JsonValue::String(value));
            }
            "upmbps" | "up" if !value.is_empty() => {
                proxy.insert("up".into(), JsonValue::String(value));
            }
            "downmbps" | "down" if !value.is_empty() => {
                proxy.insert("down".into(), JsonValue::String(value));
            }
            "fast-open" => {
                proxy.insert("fast-open".into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            "peer" | "sni" if !value.is_empty() => {
                proxy.insert("sni".into(), JsonValue::String(value));
            }
            "recv-window-conn" => {
                if let Some(number) = parse_integer(Some(&value)) {
                    proxy.insert("recv-window-conn".into(), JsonValue::Number(number.into()));
                }
            }
            "recv-window" => {
                if let Some(number) = parse_integer(Some(&value)) {
                    proxy.insert("recv-window".into(), JsonValue::Number(number.into()));
                }
            }
            "ca" if !value.is_empty() => {
                proxy.insert("ca".into(), JsonValue::String(value));
            }
            "ca-str" if !value.is_empty() => {
                proxy.insert("ca-str".into(), JsonValue::String(value));
            }
            "disable-mtu-discovery" => {
                proxy.insert(
                    "disable-mtu-discovery".into(),
                    JsonValue::Bool(boolish_value(Some(&value))),
                );
            }
            "fingerprint" if !value.is_empty() => {
                proxy.insert("fingerprint".into(), JsonValue::String(value));
            }
            "protocol" if !value.is_empty() => {
                proxy.insert("protocol".into(), JsonValue::String(value));
            }
            _ => {}
        }
    }

    if !proxy.contains_key("protocol") {
        proxy.insert("protocol".into(), JsonValue::String("udp".into()));
    }

    Ok(proxy)
}

fn parse_hysteria2(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid hysteria2 uri")?;
    let server = parsed.host_str().context("invalid hysteria2 uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let password = percent_decode(parsed.username());
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("Hysteria2 {server}:{port}"));

    let mut proxy = base_proxy_map("hysteria2", name, server, port);
    proxy.insert("password".into(), JsonValue::String(password));

    if let Some(sni) = params
        .get("sni")
        .or_else(|| params.get("peer"))
        .filter(|value| !value.is_empty())
    {
        proxy.insert("sni".into(), JsonValue::String(sni.clone()));
    }
    if let Some(obfs) = params.get("obfs").filter(|value| value.as_str() != "none") {
        proxy.insert("obfs".into(), JsonValue::String(obfs.clone()));
    }
    if let Some(mport) = params.get("mport").filter(|value| !value.is_empty()) {
        proxy.insert("ports".into(), JsonValue::String(mport.clone()));
    }
    if let Some(obfs_password) = params.get("obfs-password").filter(|value| !value.is_empty()) {
        proxy.insert("obfs-password".into(), JsonValue::String(obfs_password.clone()));
    }
    if params.contains_key("insecure") {
        proxy.insert(
            "skip-cert-verify".into(),
            JsonValue::Bool(boolish_value(params.get("insecure"))),
        );
    }
    if params.contains_key("fastopen") {
        proxy.insert("tfo".into(), JsonValue::Bool(boolish_value(params.get("fastopen"))));
    }
    if let Some(fingerprint) = params.get("pinSHA256").filter(|value| !value.is_empty()) {
        proxy.insert("fingerprint".into(), JsonValue::String(fingerprint.clone()));
    }

    Ok(proxy)
}

fn parse_tuic(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid tuic uri")?;
    let server = parsed.host_str().context("invalid tuic uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let auth = percent_decode(parsed.username());
    let (uuid, password) = auth.split_once(':').context("invalid tuic uri: missing password")?;
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("TUIC {server}:{port}"));

    let mut proxy = base_proxy_map("tuic", name, server, port);
    proxy.insert("uuid".into(), JsonValue::String(uuid.to_owned()));
    proxy.insert("password".into(), JsonValue::String(password.to_owned()));

    for (key, value) in params {
        match key.as_str() {
            "token" | "ip" | "udp-relay-mode" | "congestion-controller" | "sni" if !value.is_empty() => {
                proxy.insert(key, JsonValue::String(value));
            }
            "heartbeat-interval" | "request-timeout" | "max-udp-relay-packet-size" | "max-open-streams" => {
                if let Some(number) = parse_integer(Some(&value)) {
                    proxy.insert(key, JsonValue::Number(number.into()));
                }
            }
            "alpn" => {
                let values = split_csv(&value);
                if !values.is_empty() {
                    proxy.insert(
                        "alpn".into(),
                        JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
                    );
                }
            }
            "disable-sni" | "reduce-rtt" | "fast-open" | "skip-cert-verify" | "allow-insecure" => {
                let target = if key == "allow-insecure" {
                    "skip-cert-verify"
                } else {
                    key.as_str()
                };
                proxy.insert(target.into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            _ => {}
        }
    }

    Ok(proxy)
}

fn parse_http(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid http uri")?;
    let server = parsed.host_str().context("invalid http uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("HTTP {server}:{port}"));

    let mut proxy = base_proxy_map("http", name, server, port);
    if !parsed.username().is_empty() {
        proxy.insert("username".into(), JsonValue::String(percent_decode(parsed.username())));
    }
    if let Some(password) = parsed.password() {
        proxy.insert("password".into(), JsonValue::String(percent_decode(password)));
    }

    for (key, value) in params {
        match key.as_str() {
            "tls" => {
                proxy.insert("tls".into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            "fingerprint" if !value.is_empty() => {
                proxy.insert("fingerprint".into(), JsonValue::String(value));
            }
            "skip-cert-verify" => {
                proxy.insert("skip-cert-verify".into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            "ip-version" if !value.is_empty() => {
                proxy.insert(
                    "ip-version".into(),
                    JsonValue::String(normalize_ip_version(&value).into()),
                );
            }
            _ => {}
        }
    }

    Ok(proxy)
}

fn parse_socks(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid socks uri")?;
    let server = parsed.host_str().context("invalid socks uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("SOCKS5 {server}:{port}"));

    let mut proxy = base_proxy_map("socks5", name, server, port);
    if !parsed.username().is_empty() {
        proxy.insert("username".into(), JsonValue::String(percent_decode(parsed.username())));
    }
    if let Some(password) = parsed.password() {
        proxy.insert("password".into(), JsonValue::String(percent_decode(password)));
    }

    for (key, value) in params {
        match key.as_str() {
            "tls" => {
                proxy.insert("tls".into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            "fingerprint" if !value.is_empty() => {
                proxy.insert("fingerprint".into(), JsonValue::String(value));
            }
            "skip-cert-verify" => {
                proxy.insert("skip-cert-verify".into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            "udp" => {
                proxy.insert("udp".into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            "ip-version" if !value.is_empty() => {
                proxy.insert(
                    "ip-version".into(),
                    JsonValue::String(normalize_ip_version(&value).into()),
                );
            }
            _ => {}
        }
    }

    Ok(proxy)
}

fn parse_wireguard(line: &str) -> Result<JsonMap<String, JsonValue>> {
    let parsed = Url::parse(line).context("invalid wireguard uri")?;
    let server = parsed.host_str().context("invalid wireguard uri: missing host")?;
    let port = parsed.port().unwrap_or(443);
    let params = query_pairs_map(&parsed, true);
    let name = decode_and_trim(parsed.fragment()).unwrap_or_else(|| format!("WireGuard {server}:{port}"));

    let mut proxy = base_proxy_map("wireguard", name, server, port);
    if !parsed.username().is_empty() {
        proxy.insert(
            "private-key".into(),
            JsonValue::String(percent_decode(parsed.username())),
        );
    }
    proxy.insert("udp".into(), JsonValue::Bool(true));

    for (key, value) in params {
        match key.as_str() {
            "address" | "ip" => {
                for item in value.split(',') {
                    let ip = item
                        .trim()
                        .trim_start_matches('[')
                        .trim_end_matches(']')
                        .split('/')
                        .next()
                        .unwrap_or_default();
                    if is_ipv4(ip) {
                        proxy.insert("ip".into(), JsonValue::String(ip.to_owned()));
                    } else if is_ipv6(ip) {
                        proxy.insert("ipv6".into(), JsonValue::String(ip.to_owned()));
                    }
                }
            }
            "publickey" | "public-key" if !value.is_empty() => {
                proxy.insert("public-key".into(), JsonValue::String(value));
            }
            "allowed-ips" if !value.is_empty() => {
                proxy.insert(
                    "allowed-ips".into(),
                    JsonValue::Array(split_csv(&value).into_iter().map(JsonValue::String).collect()),
                );
            }
            "pre-shared-key" if !value.is_empty() => {
                proxy.insert("pre-shared-key".into(), JsonValue::String(value));
            }
            "reserved" => {
                let numbers = value
                    .split(',')
                    .filter_map(|item| parse_integer(Some(item.trim())))
                    .collect::<Vec<_>>();
                if numbers.len() == 3 {
                    proxy.insert(
                        "reserved".into(),
                        JsonValue::Array(
                            numbers
                                .into_iter()
                                .map(|number| JsonValue::Number(number.into()))
                                .collect(),
                        ),
                    );
                }
            }
            "udp" => {
                proxy.insert("udp".into(), JsonValue::Bool(boolish_value(Some(&value))));
            }
            "mtu" => {
                if let Some(number) = parse_integer(Some(&value)) {
                    proxy.insert("mtu".into(), JsonValue::Number(number.into()));
                }
            }
            "dialer-proxy" if !value.is_empty() => {
                proxy.insert("dialer-proxy".into(), JsonValue::String(value));
            }
            "remote-dns-resolve" => {
                proxy.insert(
                    "remote-dns-resolve".into(),
                    JsonValue::Bool(boolish_value(Some(&value))),
                );
            }
            "dns" if !value.is_empty() => {
                proxy.insert(
                    "dns".into(),
                    JsonValue::Array(split_csv(&value).into_iter().map(JsonValue::String).collect()),
                );
            }
            _ => {}
        }
    }

    Ok(proxy)
}

fn strip_uri_scheme<'a>(uri: &'a str, expected: &[&str]) -> Result<&'a str> {
    let Some((scheme, rest)) = uri.trim().split_once("://") else {
        bail!("invalid uri");
    };

    let scheme = scheme.to_ascii_lowercase();
    if !expected.iter().any(|item| *item == scheme) {
        bail!("invalid uri scheme: {scheme}");
    }

    Ok(rest)
}

fn split_once<'a>(input: &'a str, delimiter: char) -> (&'a str, Option<&'a str>) {
    match input.split_once(delimiter) {
        Some((left, right)) => (left, Some(right)),
        None => (input, None),
    }
}

fn parse_required_port(value: impl AsRef<str>, error_message: &str) -> Result<u16> {
    let raw = value.as_ref().trim();
    if raw.is_empty() || !raw.chars().all(|ch| ch.is_ascii_digit()) {
        bail!(error_message.to_owned());
    }
    let parsed = raw
        .parse::<u16>()
        .map_err(|_| anyhow::anyhow!(error_message.to_owned()))?;
    if parsed == 0 {
        bail!(error_message.to_owned());
    }
    Ok(parsed)
}

fn parse_required_port_opt(value: Option<&str>, error_message: &str) -> Result<u16> {
    parse_required_port(value.context(error_message.to_owned())?, error_message)
}

fn parse_integer(value: Option<&str>) -> Option<u64> {
    value.and_then(|raw| raw.trim().parse::<u64>().ok())
}

fn parse_query_string(query: Option<&str>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(query) = query else {
        return out;
    };

    for part in query.split('&') {
        if part.is_empty() {
            continue;
        }
        let (key, value) = split_once(part, '=');
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        out.insert(key.to_owned(), percent_decode(value.unwrap_or_default()));
    }

    out
}

fn query_pairs_map(url: &Url, normalize_underscores: bool) -> HashMap<String, String> {
    let mut params = HashMap::new();
    for (key, value) in url.query_pairs() {
        let key = if normalize_underscores {
            key.replace('_', "-")
        } else {
            key.into_owned()
        };
        params.insert(key, value.into_owned());
    }
    params
}

fn decode_base64_or_original(value: &str) -> String {
    let normalized = value
        .chars()
        .filter(|ch| !matches!(ch, '\r' | '\n' | '\t' | ' '))
        .collect::<String>()
        .replace('-', "+")
        .replace('_', "/");

    let padded = match normalized.len() % 4 {
        0 => normalized.clone(),
        rem => {
            let mut value = normalized.clone();
            value.push_str(&"=".repeat(4 - rem));
            value
        }
    };

    for engine in [&STANDARD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        if let Ok(bytes) = engine.decode(padded.as_bytes()) {
            if bytes
                .iter()
                .all(|byte| matches!(*byte, b'\t' | b'\n' | b'\r') || (*byte >= 32 && *byte != 127))
                && let Ok(text) = String::from_utf8(bytes)
            {
                return text;
            }
        }
    }

    value.to_owned()
}

fn percent_decode(value: &str) -> String {
    percent_encoding::percent_decode_str(value)
        .decode_utf8_lossy()
        .to_string()
}

fn decode_and_trim(value: Option<&str>) -> Option<String> {
    value
        .map(percent_decode)
        .map(|decoded| decoded.trim().to_owned())
        .filter(|decoded| !decoded.is_empty())
}

fn normalize_cipher(value: &str) -> String {
    match value {
        "chacha20-poly1305" => "chacha20-ietf-poly1305".into(),
        "" => "none".into(),
        other => other.to_owned(),
    }
}

fn normalize_ip_version(value: &str) -> &str {
    match value {
        "ipv4" | "ipv6" | "ipv4-prefer" | "ipv6-prefer" => value,
        _ => "dual",
    }
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(std::borrow::ToOwned::to_owned)
        .collect()
}

fn boolish_key_present(params: &HashMap<String, String>, key: &str) -> bool {
    params.contains_key(key) && boolish_value(params.get(key))
}

fn boolish_value(value: Option<&String>) -> bool {
    match value {
        None => true,
        Some(value) => {
            let trimmed = value.trim();
            trimmed.is_empty() || matches!(trimmed, "1" | "true" | "TRUE" | "True")
        }
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "1" | "true" | "TRUE" | "True" => Some(true),
        "0" | "false" | "FALSE" | "False" => Some(false),
        _ => None,
    }
}

fn normalize_upper_enum(value: Option<&String>, allowed: &[&str]) -> Option<String> {
    let normalized = value?.trim().to_ascii_uppercase();
    allowed
        .iter()
        .copied()
        .find(|candidate| normalized == *candidate)
        .map(str::to_owned)
}

fn normalize_lower_enum(value: Option<&String>, allowed: &[&str]) -> Option<String> {
    let normalized = value?.trim().to_ascii_lowercase();
    allowed
        .iter()
        .copied()
        .find(|candidate| normalized == *candidate)
        .map(str::to_owned)
}

fn is_valid_vless_flow(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && !trimmed.eq_ignore_ascii_case("none")
        && trimmed.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
}

fn base_proxy_map(proxy_type: &str, name: String, server: &str, port: u16) -> JsonMap<String, JsonValue> {
    let mut proxy = JsonMap::new();
    proxy.insert("name".into(), JsonValue::String(name));
    proxy.insert("type".into(), JsonValue::String(proxy_type.to_owned()));
    proxy.insert("server".into(), JsonValue::String(server.to_owned()));
    proxy.insert("port".into(), JsonValue::Number(port.into()));
    proxy
}

fn transport_host_json_host(value: &str) -> Option<String> {
    serde_json::from_str::<JsonValue>(value).ok().and_then(|parsed| {
        parsed
            .get("Host")
            .and_then(JsonValue::as_str)
            .map(std::borrow::ToOwned::to_owned)
    })
}

fn looks_like_external_profile(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }

    (trimmed.starts_with('{') && trimmed.contains("\"outbounds\""))
        || trimmed.contains("[Proxy]")
        || trimmed.contains("[server_local]")
        || trimmed.contains("[server_remote]")
        || trimmed.contains("custom_proxy_group=")
}

fn normalize_external_profile(input: &str) -> Result<Option<NormalizedSubscription>> {
    if let Some(config) = translate_json_profile(input)? {
        return build_translated_subscription(config).map(Some);
    }

    if let Some(config) = translate_ini_profile(input)? {
        return build_translated_subscription(config).map(Some);
    }

    Ok(None)
}

fn build_translated_subscription(config: TranslatedConfig) -> Result<NormalizedSubscription> {
    let suggested_name = match config.proxies.len() {
        0 => None,
        1 => config.proxies[0]
            .get("name")
            .and_then(JsonValue::as_str)
            .map(std::borrow::ToOwned::to_owned),
        len => config.proxies[0]
            .get("name")
            .and_then(JsonValue::as_str)
            .map(|name| format!("{name} +{}", len - 1)),
    };

    Ok(NormalizedSubscription {
        yaml: generate_translated_yaml(config)?,
        suggested_name,
    })
}

fn translate_json_profile(input: &str) -> Result<Option<TranslatedConfig>> {
    let Ok(json) = serde_json::from_str::<JsonValue>(input) else {
        return Ok(None);
    };
    let Some(root) = json.as_object() else {
        return Ok(None);
    };
    let Some(outbounds) = root.get("outbounds").and_then(JsonValue::as_array) else {
        return Ok(None);
    };

    if outbounds.iter().any(|outbound| outbound.get("type").is_some()) {
        let mut config = parse_sing_box_outbounds(outbounds);
        if let Some(route) = root.get("route").and_then(JsonValue::as_object) {
            config.warnings.extend(collect_sing_box_route_warnings(route));
            config.rules.extend(parse_sing_box_route_rules(route));
            config.rule_providers.extend(parse_sing_box_rule_set_providers(route));
        }
        if let Some(rule_sets) = root
            .get("rule_set")
            .or_else(|| root.get("rule-set"))
            .and_then(JsonValue::as_array)
        {
            config.rule_providers.extend(parse_sing_box_rule_set_array(rule_sets));
        }
        return Ok(Some(config));
    }

    if outbounds.iter().any(|outbound| outbound.get("protocol").is_some()) {
        let mut config = parse_xray_outbounds(outbounds);
        if let Some(routing) = root.get("routing").and_then(JsonValue::as_object) {
            config.warnings.extend(collect_xray_routing_warnings(routing));
            config.rules.extend(parse_xray_routing_rules(routing));
            config
                .proxy_groups
                .extend(parse_xray_balancers(routing, &config.proxies));
        }
        return Ok(Some(config));
    }

    Ok(None)
}

fn parse_sing_box_outbounds(outbounds: &[JsonValue]) -> TranslatedConfig {
    let mut config = TranslatedConfig::default();

    for outbound in outbounds.iter().filter_map(JsonValue::as_object) {
        let Some(outbound_type) = outbound.get("type").and_then(JsonValue::as_str) else {
            continue;
        };

        match outbound_type {
            "direct" | "block" | "dns" => continue,
            _ => {}
        }

        let name = outbound
            .get("tag")
            .and_then(JsonValue::as_str)
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_else(|| outbound_type.to_ascii_uppercase());

        let maybe_proxy = match outbound_type {
            "shadowsocks" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("ss", name, server, port);
                insert_string_field(&mut proxy, "cipher", outbound.get("method"));
                insert_string_field(&mut proxy, "password", outbound.get("password"));
                Some(proxy)
            }
            "socks" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("socks5", name, server, port);
                insert_string_field(&mut proxy, "username", outbound.get("username"));
                insert_string_field(&mut proxy, "password", outbound.get("password"));
                Some(proxy)
            }
            "http" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("http", name, server, port);
                insert_string_field(&mut proxy, "username", outbound.get("username"));
                insert_string_field(&mut proxy, "password", outbound.get("password"));
                Some(proxy)
            }
            "vmess" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("vmess", name, server, port);
                insert_string_field(&mut proxy, "uuid", outbound.get("uuid"));
                insert_string_field(&mut proxy, "cipher", outbound.get("security"));
                if let Some(alter_id) = outbound.get("alter_id").and_then(value_as_u64) {
                    proxy.insert("alterId".into(), JsonValue::Number(alter_id.into()));
                }
                Some(proxy)
            }
            "vless" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("vless", name, server, port);
                insert_string_field(&mut proxy, "uuid", outbound.get("uuid"));
                insert_string_field(&mut proxy, "flow", outbound.get("flow"));
                Some(proxy)
            }
            "trojan" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("trojan", name, server, port);
                insert_string_field(&mut proxy, "password", outbound.get("password"));
                Some(proxy)
            }
            "anytls" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("anytls", name, server, port);
                insert_string_field(&mut proxy, "password", outbound.get("password"));
                proxy.insert("udp".into(), JsonValue::Bool(true));
                Some(proxy)
            }
            "ssh" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("ssh", name, server, port);
                insert_string_field(
                    &mut proxy,
                    "username",
                    outbound.get("user").or_else(|| outbound.get("username")),
                );
                insert_string_field(&mut proxy, "password", outbound.get("password"));
                insert_string_field(
                    &mut proxy,
                    "private-key",
                    outbound.get("private_key").or_else(|| outbound.get("privateKey")),
                );
                insert_string_field(
                    &mut proxy,
                    "private-key-passphrase",
                    outbound
                        .get("private_key_passphrase")
                        .or_else(|| outbound.get("privateKeyPassphrase")),
                );
                insert_string_field(
                    &mut proxy,
                    "host-key",
                    outbound.get("host_key").or_else(|| outbound.get("hostKey")),
                );
                insert_string_field(
                    &mut proxy,
                    "host-key-algorithms",
                    outbound
                        .get("host_key_algorithms")
                        .or_else(|| outbound.get("hostKeyAlgorithms")),
                );
                Some(proxy)
            }
            "hysteria" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("hysteria", name, server, port);
                insert_string_field(
                    &mut proxy,
                    "auth-str",
                    outbound.get("auth_str").or_else(|| outbound.get("auth")),
                );
                insert_string_field(&mut proxy, "obfs", outbound.get("obfs"));
                insert_string_field(&mut proxy, "up", outbound.get("up_mbps").or_else(|| outbound.get("up")));
                insert_string_field(
                    &mut proxy,
                    "down",
                    outbound.get("down_mbps").or_else(|| outbound.get("down")),
                );
                Some(proxy)
            }
            "hysteria2" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("hysteria2", name, server, port);
                insert_string_field(&mut proxy, "password", outbound.get("password"));
                insert_string_field(&mut proxy, "obfs", outbound.get("obfs"));
                insert_string_field(
                    &mut proxy,
                    "obfs-password",
                    outbound.get("obfs_password").or_else(|| outbound.get("obfs-password")),
                );
                insert_string_field(&mut proxy, "up", outbound.get("up_mbps").or_else(|| outbound.get("up")));
                insert_string_field(
                    &mut proxy,
                    "down",
                    outbound.get("down_mbps").or_else(|| outbound.get("down")),
                );
                Some(proxy)
            }
            "tuic" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("tuic", name, server, port);
                insert_string_field(&mut proxy, "uuid", outbound.get("uuid"));
                insert_string_field(&mut proxy, "password", outbound.get("password"));
                insert_string_field(&mut proxy, "token", outbound.get("token"));
                insert_string_field(&mut proxy, "congestion-controller", outbound.get("congestion_control"));
                Some(proxy)
            }
            "wireguard" => {
                let Some(server) = outbound.get("server").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = outbound.get("server_port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("wireguard", name, server, port);
                insert_string_field(
                    &mut proxy,
                    "private-key",
                    outbound.get("private_key").or_else(|| outbound.get("secret_key")),
                );
                insert_string_field(
                    &mut proxy,
                    "public-key",
                    outbound.get("peer_public_key").or_else(|| outbound.get("public_key")),
                );
                insert_string_field(&mut proxy, "pre-shared-key", outbound.get("pre_shared_key"));
                if let Some(addresses) = outbound.get("local_address").and_then(JsonValue::as_array) {
                    for address in addresses.iter().filter_map(JsonValue::as_str) {
                        let ip = address.split('/').next().unwrap_or_default();
                        if is_ipv4(ip) {
                            proxy.insert("ip".into(), JsonValue::String(ip.to_owned()));
                        } else if is_ipv6(ip) {
                            proxy.insert("ipv6".into(), JsonValue::String(ip.to_owned()));
                        }
                    }
                }
                proxy.insert("udp".into(), JsonValue::Bool(true));
                Some(proxy)
            }
            "selector" | "urltest" => {
                if let Some(group) = parse_sing_box_group(outbound_type, &name, outbound) {
                    config.proxy_groups.push(group);
                }
                None
            }
            _ => None,
        };

        if let Some(mut proxy) = maybe_proxy {
            apply_tls_object(&mut proxy, outbound.get("tls"));
            apply_transport_object(&mut proxy, outbound.get("transport"));
            config.proxies.push(proxy);
        }
    }

    config
}

fn parse_xray_outbounds(outbounds: &[JsonValue]) -> TranslatedConfig {
    let mut config = TranslatedConfig::default();

    for outbound in outbounds.iter().filter_map(JsonValue::as_object) {
        let Some(protocol) = outbound.get("protocol").and_then(JsonValue::as_str) else {
            continue;
        };

        match protocol {
            "freedom" | "blackhole" | "dns" => continue,
            _ => {}
        }

        let name = outbound
            .get("tag")
            .and_then(JsonValue::as_str)
            .map(std::borrow::ToOwned::to_owned)
            .unwrap_or_else(|| protocol.to_ascii_uppercase());

        let Some(settings) = outbound.get("settings").and_then(JsonValue::as_object) else {
            continue;
        };

        let maybe_proxy = match protocol {
            "shadowsocks" => {
                let Some(server) = first_array_object(settings.get("servers")) else {
                    continue;
                };
                let Some(address) = server.get("address").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = server.get("port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("ss", name, address, port);
                insert_string_field(&mut proxy, "cipher", server.get("method"));
                insert_string_field(&mut proxy, "password", server.get("password"));
                Some(proxy)
            }
            "socks" => {
                let Some(server) = first_array_object(settings.get("servers")) else {
                    continue;
                };
                let Some(address) = server.get("address").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = server.get("port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("socks5", name, address, port);
                if let Some(user) = first_array_object(server.get("users")) {
                    insert_string_field(&mut proxy, "username", user.get("user"));
                    insert_string_field(&mut proxy, "password", user.get("pass"));
                }
                Some(proxy)
            }
            "http" => {
                let Some(server) = first_array_object(settings.get("servers")) else {
                    continue;
                };
                let Some(address) = server.get("address").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = server.get("port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("http", name, address, port);
                if let Some(user) = first_array_object(server.get("users")) {
                    insert_string_field(&mut proxy, "username", user.get("user"));
                    insert_string_field(&mut proxy, "password", user.get("pass"));
                }
                Some(proxy)
            }
            "vmess" | "vless" => {
                let Some(vnext) = first_array_object(settings.get("vnext")) else {
                    continue;
                };
                let Some(address) = vnext.get("address").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = vnext.get("port").and_then(value_as_u16) else {
                    continue;
                };
                let Some(user) = first_array_object(vnext.get("users")) else {
                    continue;
                };
                let mut proxy = base_proxy_map(protocol, name, address, port);
                insert_string_field(&mut proxy, "uuid", user.get("id"));
                if protocol == "vmess" {
                    insert_string_field(&mut proxy, "cipher", user.get("security"));
                    if let Some(alter_id) = user.get("alterId").and_then(value_as_u64) {
                        proxy.insert("alterId".into(), JsonValue::Number(alter_id.into()));
                    }
                } else {
                    insert_string_field(&mut proxy, "flow", user.get("flow"));
                }
                Some(proxy)
            }
            "trojan" => {
                let Some(server) = first_array_object(settings.get("servers")) else {
                    continue;
                };
                let Some(address) = server.get("address").and_then(JsonValue::as_str) else {
                    continue;
                };
                let Some(port) = server.get("port").and_then(value_as_u16) else {
                    continue;
                };
                let mut proxy = base_proxy_map("trojan", name, address, port);
                insert_string_field(&mut proxy, "password", server.get("password"));
                Some(proxy)
            }
            "wireguard" => {
                let mut proxy = base_proxy_map(
                    "wireguard",
                    name,
                    settings
                        .get("address")
                        .and_then(JsonValue::as_str)
                        .unwrap_or("127.0.0.1"),
                    settings.get("port").and_then(value_as_u16).unwrap_or(51820),
                );
                insert_string_field(
                    &mut proxy,
                    "private-key",
                    settings.get("secretKey").or_else(|| settings.get("privateKey")),
                );
                if let Some(peer) = first_array_object(settings.get("peers")) {
                    if let Some(endpoint) = peer.get("endpoint").and_then(JsonValue::as_str) {
                        if let Some((server, port)) = split_host_port(endpoint) {
                            proxy.insert("server".into(), JsonValue::String(server));
                            proxy.insert("port".into(), JsonValue::Number((port as u64).into()));
                        }
                    }
                    insert_string_field(
                        &mut proxy,
                        "public-key",
                        peer.get("publicKey").or_else(|| peer.get("peerPublicKey")),
                    );
                }
                if let Some(addresses) = settings.get("address").and_then(JsonValue::as_array) {
                    for address in addresses.iter().filter_map(JsonValue::as_str) {
                        let ip = address.split('/').next().unwrap_or_default();
                        if is_ipv4(ip) {
                            proxy.insert("ip".into(), JsonValue::String(ip.to_owned()));
                        } else if is_ipv6(ip) {
                            proxy.insert("ipv6".into(), JsonValue::String(ip.to_owned()));
                        }
                    }
                }
                proxy.insert("udp".into(), JsonValue::Bool(true));
                Some(proxy)
            }
            _ => None,
        };

        if let Some(mut proxy) = maybe_proxy {
            apply_xray_stream_settings(&mut proxy, outbound.get("streamSettings"));
            config.proxies.push(proxy);
        }
    }

    config
}

fn parse_sing_box_group(
    outbound_type: &str,
    name: &str,
    outbound: &serde_json::Map<String, JsonValue>,
) -> Option<JsonMap<String, JsonValue>> {
    let proxies = outbound
        .get("outbounds")
        .and_then(JsonValue::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(JsonValue::as_str)
                .map(std::borrow::ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty())?;

    let group_type = match outbound_type {
        "selector" => "select",
        "urltest" => "url-test",
        _ => return None,
    };

    let mut group = JsonMap::new();
    group.insert("name".into(), JsonValue::String(name.to_owned()));
    group.insert("type".into(), JsonValue::String(group_type.into()));
    group.insert(
        "proxies".into(),
        JsonValue::Array(proxies.into_iter().map(JsonValue::String).collect()),
    );

    if outbound_type == "urltest" {
        if let Some(url) = outbound.get("url").and_then(JsonValue::as_str) {
            group.insert("url".into(), JsonValue::String(url.to_owned()));
        }
        if let Some(interval) = outbound
            .get("interval")
            .and_then(value_as_string_ref)
            .and_then(parse_duration_like)
        {
            group.insert("interval".into(), JsonValue::Number(interval.into()));
        }
        if let Some(tolerance) = outbound
            .get("tolerance")
            .and_then(value_as_u64)
            .or_else(|| outbound.get("idle_timeout").and_then(value_as_u64))
        {
            group.insert("tolerance".into(), JsonValue::Number(tolerance.into()));
        }
    }

    Some(group)
}

fn parse_sing_box_route_rules(route: &serde_json::Map<String, JsonValue>) -> Vec<String> {
    let Some(rules) = route.get("rules").and_then(JsonValue::as_array) else {
        return Vec::new();
    };

    let mut translated = Vec::new();
    for rule in rules.iter().filter_map(JsonValue::as_object) {
        let Some(target) = sing_box_rule_target(rule) else {
            continue;
        };
        translated.extend(parse_common_json_rules(rule, &target));
    }
    translated
}

fn parse_sing_box_rule_set_providers(route: &serde_json::Map<String, JsonValue>) -> JsonMap<String, JsonValue> {
    if let Some(rule_sets) = route
        .get("rule_set")
        .or_else(|| route.get("rule-set"))
        .and_then(JsonValue::as_array)
    {
        parse_sing_box_rule_set_array(rule_sets)
    } else {
        JsonMap::new()
    }
}

fn parse_sing_box_rule_set_array(rule_sets: &[JsonValue]) -> JsonMap<String, JsonValue> {
    let mut providers = JsonMap::new();

    for rule_set in rule_sets.iter().filter_map(JsonValue::as_object) {
        let Some(tag) = rule_set.get("tag").and_then(JsonValue::as_str) else {
            continue;
        };

        let source = rule_set
            .get("url")
            .and_then(JsonValue::as_str)
            .or_else(|| rule_set.get("path").and_then(JsonValue::as_str));
        let Some(source) = source else {
            continue;
        };

        let interval = rule_set
            .get("update_interval")
            .or_else(|| rule_set.get("update-interval"))
            .and_then(value_as_string_ref)
            .and_then(parse_duration_like)
            .or_else(|| rule_set.get("update_interval").and_then(value_as_u64))
            .or_else(|| rule_set.get("update-interval").and_then(value_as_u64));
        let mut provider = build_rule_provider_entry(tag, source, interval);

        if let Some(detour) = rule_set
            .get("download_detour")
            .or_else(|| rule_set.get("download-detour"))
            .and_then(JsonValue::as_str)
        {
            provider.insert("proxy".into(), JsonValue::String(detour.to_owned()));
        }

        providers.insert(tag.to_owned(), JsonValue::Object(provider));
    }

    providers
}

fn parse_xray_routing_rules(routing: &serde_json::Map<String, JsonValue>) -> Vec<String> {
    let Some(rules) = routing.get("rules").and_then(JsonValue::as_array) else {
        return Vec::new();
    };

    let mut translated = Vec::new();
    for rule in rules.iter().filter_map(JsonValue::as_object) {
        let Some(target) = rule
            .get("outboundTag")
            .and_then(JsonValue::as_str)
            .or_else(|| rule.get("balancerTag").and_then(JsonValue::as_str))
        else {
            continue;
        };

        let mut matchers = Vec::new();

        let domains = json_string_values(rule.get("domain"));
        if !domains.is_empty() {
            let domain_rules = domains
                .iter()
                .map(String::as_str)
                .filter_map(|value| parse_xray_domain_rule(value, target))
                .collect::<Vec<_>>();
            if !domain_rules.is_empty() {
                matchers.push(domain_rules);
            }
        }

        let ips = json_string_values(rule.get("ip"));
        if !ips.is_empty() {
            let ip_rules = ips
                .iter()
                .map(String::as_str)
                .filter_map(|value| parse_xray_ip_rule(value, target))
                .collect::<Vec<_>>();
            if !ip_rules.is_empty() {
                matchers.push(ip_rules);
            }
        }

        if let Some(network) = rule.get("network").and_then(JsonValue::as_str) {
            let network_rules = network
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("NETWORK,{value},{target}"))
                .collect::<Vec<_>>();
            if !network_rules.is_empty() {
                matchers.push(network_rules);
            }
        }
        if let Some(ports) = rule.get("port").and_then(JsonValue::as_str) {
            let rules = ports
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("DST-PORT,{value},{target}"))
                .collect::<Vec<_>>();
            if !rules.is_empty() {
                matchers.push(rules);
            }
        }
        if let Some(local_ports) = rule
            .get("localPort")
            .or_else(|| rule.get("local_port"))
            .and_then(JsonValue::as_str)
        {
            let rules = local_ports
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("IN-PORT,{value},{target}"))
                .collect::<Vec<_>>();
            if !rules.is_empty() {
                matchers.push(rules);
            }
        }
        let processes = json_string_values(rule.get("process"));
        if !processes.is_empty() {
            let process_rules = processes
                .iter()
                .map(String::as_str)
                .map(|value| format!("PROCESS-NAME,{value},{target}"))
                .collect::<Vec<_>>();
            if !process_rules.is_empty() {
                matchers.push(process_rules);
            }
        }
        if let Some(source_ports) = rule
            .get("sourcePort")
            .or_else(|| rule.get("source_port"))
            .and_then(JsonValue::as_str)
        {
            let rules = source_ports
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(|value| format!("SRC-PORT,{value},{target}"))
                .collect::<Vec<_>>();
            if !rules.is_empty() {
                matchers.push(rules);
            }
        }
        let source_ips = json_string_values(
            rule.get("sourceIP")
                .or_else(|| rule.get("source"))
                .or_else(|| rule.get("source_ip")),
        );
        if !source_ips.is_empty() {
            let src_rules = source_ips
                .iter()
                .map(String::as_str)
                .filter_map(|value| {
                    if value.starts_with("geoip:") {
                        value
                            .strip_prefix("geoip:")
                            .map(|payload| format!("GEOIP,{payload},{target}"))
                    } else if let Some(value) = exact_ip_cidr(value) {
                        Some(format!("SRC-IP-CIDR,{value},{target}"))
                    } else if value.contains('/') {
                        Some(format!("SRC-IP-CIDR,{value},{target}"))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();
            if !src_rules.is_empty() {
                matchers.push(src_rules);
            }
        }
        let users = json_string_values(rule.get("user"));
        if !users.is_empty() {
            let user_rules = users
                .iter()
                .map(String::as_str)
                .map(|value| format!("IN-USER,{value},{target}"))
                .collect::<Vec<_>>();
            if !user_rules.is_empty() {
                matchers.push(user_rules);
            }
        }
        let protocols = json_string_values(rule.get("protocol"));
        if !protocols.is_empty() {
            let protocol_rules = protocols
                .iter()
                .map(String::as_str)
                .map(|value| format!("IN-TYPE,{value},{target}"))
                .collect::<Vec<_>>();
            if !protocol_rules.is_empty() {
                matchers.push(protocol_rules);
            }
        }
        let inbound_tags = json_string_values(rule.get("inboundTag"));
        if !inbound_tags.is_empty() {
            let inbound_rules = inbound_tags
                .iter()
                .map(String::as_str)
                .map(|value| format!("IN-NAME,{value},{target}"))
                .collect::<Vec<_>>();
            if !inbound_rules.is_empty() {
                matchers.push(inbound_rules);
            }
        }

        translated.extend(collapse_matchers(matchers, target, false));
    }

    translated
}

fn parse_xray_balancers(
    routing: &serde_json::Map<String, JsonValue>,
    proxies: &[JsonMap<String, JsonValue>],
) -> Vec<JsonMap<String, JsonValue>> {
    let Some(balancers) = routing.get("balancers").and_then(JsonValue::as_array) else {
        return Vec::new();
    };

    let proxy_names = proxies
        .iter()
        .filter_map(|proxy| proxy.get("name").and_then(JsonValue::as_str))
        .collect::<Vec<_>>();

    let mut groups = Vec::new();
    for balancer in balancers.iter().filter_map(JsonValue::as_object) {
        let Some(name) = balancer.get("tag").and_then(JsonValue::as_str) else {
            continue;
        };
        let selectors = balancer
            .get("selector")
            .and_then(JsonValue::as_array)
            .map(|items| items.iter().filter_map(JsonValue::as_str).collect::<Vec<_>>())
            .unwrap_or_default();
        if selectors.is_empty() {
            continue;
        }

        let members = proxy_names
            .iter()
            .filter(|proxy_name| selectors.iter().any(|selector| proxy_name.starts_with(selector)))
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>();
        if members.is_empty() {
            continue;
        }

        let strategy = balancer.get("strategy").and_then(JsonValue::as_str).unwrap_or("random");
        let group_type = if strategy.eq_ignore_ascii_case("leastPing") {
            "url-test"
        } else {
            "load-balance"
        };

        let mut group = JsonMap::new();
        group.insert("name".into(), JsonValue::String(name.to_owned()));
        group.insert("type".into(), JsonValue::String(group_type.into()));
        group.insert(
            "proxies".into(),
            JsonValue::Array(members.into_iter().map(JsonValue::String).collect()),
        );
        if group_type == "url-test" {
            group.insert(
                "url".into(),
                JsonValue::String("http://www.gstatic.com/generate_204".into()),
            );
            group.insert("interval".into(), JsonValue::Number(300_u64.into()));
        }
        groups.push(group);
    }

    groups
}

fn translate_ini_profile(input: &str) -> Result<Option<TranslatedConfig>> {
    let sections = parse_ini_sections(input);
    if sections.is_empty() {
        return Ok(None);
    }

    let mut config = TranslatedConfig::default();

    if let Some(proxy_lines) = sections.get("proxy") {
        for line in proxy_lines {
            if let Some(proxy) = parse_named_or_typed_proxy_line(line)? {
                config.proxies.push(proxy);
            }
        }
    }

    if let Some(root_lines) = sections.get("") {
        for line in root_lines {
            if let Some(proxy) = parse_named_or_typed_proxy_line(line)? {
                config.proxies.push(proxy);
            }
        }
    }

    if let Some(server_local_lines) = sections.get("server_local") {
        for line in server_local_lines {
            if let Some(proxy) = parse_named_or_typed_proxy_line(line)? {
                config.proxies.push(proxy);
            }
        }
    }

    if let Some(server_remote_lines) = sections.get("server_remote") {
        for line in server_remote_lines {
            if looks_like_uri_collection(line) {
                for sub_line in line.lines().map(str::trim).filter(|line| !line.is_empty()) {
                    config.proxies.push(parse_uri(sub_line)?);
                }
            } else if let Some((name, provider)) = parse_remote_proxy_provider_line(line) {
                config.proxy_providers.insert(name, JsonValue::Object(provider));
            }
        }
    }

    let (surge_groups, surge_proxy_providers, surge_group_warnings) =
        parse_surge_proxy_groups_and_providers(sections.get("proxy group"));
    config.proxy_groups.extend(surge_groups);
    config.proxy_providers.extend(surge_proxy_providers);
    config.warnings.extend(surge_group_warnings);
    config
        .proxy_groups
        .extend(parse_quantumult_policy_groups(sections.get("policy")));
    config.proxy_groups.extend(parse_quantumult_custom_proxy_groups(input));
    let (surge_rules, surge_rule_providers, surge_rule_warnings) =
        parse_surge_rules_and_providers(sections.get("rule"));
    config.rules.extend(surge_rules);
    config.rule_providers.extend(surge_rule_providers);
    config.warnings.extend(surge_rule_warnings);
    config
        .rules
        .extend(parse_quantumult_rules(sections.get("filter_local")));
    let (qx_remote_rules, qx_rule_providers) = parse_quantumult_remote_rule_providers(sections.get("filter_remote"));
    config.rules.extend(qx_remote_rules);
    config.rule_providers.extend(qx_rule_providers);

    if config.proxies.is_empty() && config.proxy_providers.is_empty() {
        return Ok(None);
    }

    Ok(Some(config))
}

fn parse_surge_proxy_groups_and_providers(
    lines: Option<&Vec<String>>,
) -> (Vec<JsonMap<String, JsonValue>>, JsonMap<String, JsonValue>, Vec<String>) {
    let Some(lines) = lines else {
        return (Vec::new(), JsonMap::new(), Vec::new());
    };

    let mut groups = Vec::new();
    let mut providers = JsonMap::new();
    let mut warnings = Vec::new();

    for line in lines {
        if let Some((group, provider)) = parse_surge_group_line(line).ok().flatten() {
            warnings.extend(collect_surge_group_warnings(line));
            if let Some((name, provider)) = provider {
                providers.insert(name, JsonValue::Object(provider));
            }
            groups.push(group);
        }
    }

    (groups, providers, warnings)
}

fn parse_surge_group_line(
    line: &str,
) -> Result<Option<(JsonMap<String, JsonValue>, Option<(String, JsonMap<String, JsonValue>)>)>> {
    let Some((name_raw, rhs_raw)) = line.split_once('=') else {
        return Ok(None);
    };
    let tokens = split_csv_like(rhs_raw);
    if tokens.is_empty() {
        return Ok(None);
    }

    let Some(group_type) = normalize_group_type(tokens[0].trim()) else {
        return Ok(None);
    };
    let name = strip_wrapping_quotes(name_raw);
    let (positional, mut keyvals) = split_tokens(&tokens[1..]);

    let provider = if let Some(policy_path) = keyvals.get("policy-path").cloned() {
        let source = strip_wrapping_quotes(&policy_path);
        let provider_name = format!("{}_provider", sanitize_provider_name(&name));
        let interval = keyvals
            .get("interval")
            .and_then(|value| parse_duration_like(value).or_else(|| parse_integer(Some(value.as_str()))));
        let provider = build_proxy_provider_entry(&provider_name, &source, interval);
        keyvals.insert("use".into(), provider_name.clone());
        Some((provider_name, provider))
    } else {
        None
    };

    let group = build_group_from_parts(&name, group_type, &positional, &keyvals);
    Ok(group.map(|group| (group, provider)))
}

fn parse_quantumult_policy_groups(lines: Option<&Vec<String>>) -> Vec<JsonMap<String, JsonValue>> {
    let Some(lines) = lines else {
        return Vec::new();
    };

    let mut groups = Vec::new();
    for line in lines {
        let Some((kind_raw, rest_raw)) = line.split_once('=') else {
            continue;
        };

        let group_type = match kind_raw.trim().to_ascii_lowercase().as_str() {
            "static" => "select",
            "url-latency-benchmark" => "url-test",
            "available" => "fallback",
            "round-robin" => "load-balance",
            _ => continue,
        };

        let tokens = split_csv_like(rest_raw);
        if tokens.is_empty() {
            continue;
        }

        let name = strip_wrapping_quotes(&tokens[0]);
        let (positional, keyvals) = split_tokens(&tokens[1..]);
        if let Some(group) = build_group_from_parts(&name, group_type, &positional, &keyvals) {
            groups.push(group);
        }
    }

    groups
}

fn parse_quantumult_custom_proxy_groups(input: &str) -> Vec<JsonMap<String, JsonValue>> {
    let mut groups = Vec::new();

    for line in input.lines().map(str::trim) {
        let Some(payload) = line.strip_prefix("custom_proxy_group=") else {
            continue;
        };

        let parts = payload
            .split('`')
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>();
        if parts.len() < 2 {
            continue;
        }

        let Some(group_type) = normalize_qx_group_type(parts[1]) else {
            continue;
        };

        let name = strip_wrapping_quotes(parts[0]);
        let mut positional = Vec::new();
        let mut keyvals = HashMap::new();

        for part in parts.into_iter().skip(2) {
            let trimmed = part.trim();
            if let Some(policy) = trimmed.strip_prefix("[]") {
                positional.push(policy.to_owned());
            } else if looks_like_url(trimmed) {
                keyvals.insert("url".into(), trimmed.to_owned());
            } else if trimmed.chars().all(|ch| ch.is_ascii_digit()) {
                let key = if keyvals.contains_key("interval") {
                    "tolerance"
                } else {
                    "interval"
                };
                keyvals.insert(key.into(), trimmed.to_owned());
            } else if trimmed.starts_with('(') || trimmed.contains('|') {
                keyvals.insert("filter".into(), trimmed.to_owned());
            } else {
                positional.push(trimmed.to_owned());
            }
        }

        if let Some(group) = build_group_from_parts(&name, group_type, &positional, &keyvals) {
            groups.push(group);
        }
    }

    groups
}

fn parse_surge_rules_and_providers(
    lines: Option<&Vec<String>>,
) -> (Vec<String>, JsonMap<String, JsonValue>, Vec<String>) {
    let Some(lines) = lines else {
        return (Vec::new(), JsonMap::new(), Vec::new());
    };

    let mut rules = Vec::new();
    let mut providers = JsonMap::new();
    let mut warnings = Vec::new();
    for line in lines {
        if let Some((rule, provider, line_warnings)) = parse_rule_line(line) {
            warnings.extend(line_warnings);
            if let Some((name, provider)) = provider {
                providers.insert(name, JsonValue::Object(provider));
            }
            rules.push(rule);
        }
    }
    (rules, providers, warnings)
}

fn parse_quantumult_rules(lines: Option<&Vec<String>>) -> Vec<String> {
    let Some(lines) = lines else {
        return Vec::new();
    };

    lines.iter().filter_map(|line| parse_qx_rule_line(line)).collect()
}

fn parse_quantumult_remote_rule_providers(lines: Option<&Vec<String>>) -> (Vec<String>, JsonMap<String, JsonValue>) {
    let Some(lines) = lines else {
        return (Vec::new(), JsonMap::new());
    };

    let mut rules = Vec::new();
    let mut providers = JsonMap::new();

    for line in lines {
        let tokens = split_csv_like(line);
        if tokens.is_empty() {
            continue;
        }

        let source = strip_wrapping_quotes(&tokens[0]);
        if !looks_like_url(&source) && !source.contains('/') {
            continue;
        }

        let (positional, keyvals) = split_tokens(&tokens[1..]);
        if keyvals.get("enabled").is_some_and(|value| !boolish_value(Some(value))) {
            continue;
        }

        let tag = keyvals
            .get("tag")
            .map(|value| strip_wrapping_quotes(value))
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| provider_name_from_source(&source));
        let target = keyvals
            .get("force-policy")
            .or_else(|| positional.first())
            .map(|value| normalize_policy_name(value))
            .unwrap_or_default();
        if target.is_empty() {
            continue;
        }

        let interval = keyvals
            .get("update-interval")
            .and_then(|value| parse_duration_like(value).or_else(|| parse_integer(Some(value.as_str()))));
        let provider = build_rule_provider_entry(&tag, &source, interval);
        providers.insert(tag.clone(), JsonValue::Object(provider));
        rules.push(format!("RULE-SET,{tag},{target}"));
    }

    (rules, providers)
}

fn parse_remote_proxy_provider_line(line: &str) -> Option<(String, JsonMap<String, JsonValue>)> {
    let tokens = split_csv_like(line);
    if tokens.is_empty() {
        return None;
    }

    let source = strip_wrapping_quotes(&tokens[0]);
    if !looks_like_url(&source) && !source.contains('/') {
        return None;
    }

    let (positional, keyvals) = split_tokens(&tokens[1..]);
    if keyvals.get("enabled").is_some_and(|value| !boolish_value(Some(value))) {
        return None;
    }

    let name = keyvals
        .get("tag")
        .map(|value| strip_wrapping_quotes(value))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| provider_name_from_source(&source));

    let interval = keyvals
        .get("update-interval")
        .or_else(|| keyvals.get("interval"))
        .and_then(|value| parse_duration_like(value).or_else(|| parse_integer(Some(value.as_str()))));
    let mut provider = build_proxy_provider_entry(&name, &source, interval);

    if let Some(proxy) = keyvals
        .get("via-interface")
        .or_else(|| keyvals.get("via"))
        .or_else(|| positional.first())
        .filter(|value| !value.is_empty())
    {
        provider.insert("proxy".into(), JsonValue::String(normalize_policy_name(proxy)));
    }

    Some((name, provider))
}

fn sing_box_rule_target(rule: &serde_json::Map<String, JsonValue>) -> Option<String> {
    if let Some(outbound) = rule.get("outbound").and_then(JsonValue::as_str) {
        return Some(outbound.to_owned());
    }

    match rule.get("action").and_then(JsonValue::as_str) {
        Some("direct") => Some("DIRECT".into()),
        Some("reject") | Some("block") => Some("REJECT".into()),
        _ => None,
    }
}

fn parse_common_json_rules(rule: &serde_json::Map<String, JsonValue>, target: &str) -> Vec<String> {
    let mut matchers = Vec::new();

    push_json_rule_set(&mut matchers, rule, "domain", "DOMAIN", target);
    push_json_rule_set(&mut matchers, rule, "domain_suffix", "DOMAIN-SUFFIX", target);
    push_json_rule_set(&mut matchers, rule, "domain_keyword", "DOMAIN-KEYWORD", target);
    push_json_rule_set(&mut matchers, rule, "domain_regex", "DOMAIN-REGEX", target);
    push_json_rule_set(&mut matchers, rule, "geosite", "GEOSITE", target);
    push_json_rule_set(&mut matchers, rule, "geoip", "GEOIP", target);
    push_json_cidr_rules(&mut matchers, rule.get("ip_cidr"), target);
    push_json_rule_set(&mut matchers, rule, "source_ip_cidr", "SRC-IP-CIDR", target);
    push_json_private_ip_rules(&mut matchers, rule.get("ip_is_private"), "IP-CIDR", target);
    push_json_private_ip_rules(&mut matchers, rule.get("source_ip_is_private"), "SRC-IP-CIDR", target);
    push_json_rule_set(&mut matchers, rule, "process_name", "PROCESS-NAME", target);
    push_json_rule_set(&mut matchers, rule, "process_path", "PROCESS-PATH", target);
    push_json_rule_set(&mut matchers, rule, "process_path_regex", "PROCESS-PATH-REGEX", target);
    push_json_rule_set(&mut matchers, rule, "package_name", "PROCESS-NAME", target);
    push_json_rule_set(&mut matchers, rule, "package_name_regex", "PROCESS-NAME-REGEX", target);
    push_json_rule_set(&mut matchers, rule, "user", "IN-USER", target);
    push_json_rule_set(&mut matchers, rule, "user_id", "UID", target);
    push_json_rule_set(&mut matchers, rule, "clash_mode", "IN-NAME", target);
    push_json_ports(&mut matchers, rule.get("port"), "DST-PORT", target);
    push_json_ports(&mut matchers, rule.get("source_port"), "SRC-PORT", target);
    push_json_ports(&mut matchers, rule.get("source_port_range"), "SRC-PORT", target);
    push_json_rule_set(&mut matchers, rule, "rule_set", "RULE-SET", target);
    push_json_network_rules(&mut matchers, rule.get("network"), target);
    collapse_matchers(
        matchers,
        target,
        rule.get("invert").and_then(JsonValue::as_bool).unwrap_or(false),
    )
}

fn push_json_rule_set(
    matchers: &mut Vec<Vec<String>>,
    rule: &serde_json::Map<String, JsonValue>,
    key: &str,
    mihomo_type: &str,
    target: &str,
) {
    let values = json_string_values(rule.get(key));
    if !values.is_empty() {
        matchers.push(
            values
                .into_iter()
                .map(|value| format!("{mihomo_type},{value},{target}"))
                .collect(),
        );
    }
}

fn push_json_cidr_rules(matchers: &mut Vec<Vec<String>>, value: Option<&JsonValue>, target: &str) {
    let values = json_string_values(value);
    if values.is_empty() {
        return;
    }

    let mut rules = Vec::new();
    for value in values {
        let rule_type = if value.contains(':') { "IP-CIDR6" } else { "IP-CIDR" };
        rules.push(format!("{rule_type},{value},{target}"));
    }
    if !rules.is_empty() {
        matchers.push(rules);
    }
}

fn push_json_network_rules(matchers: &mut Vec<Vec<String>>, value: Option<&JsonValue>, target: &str) {
    let values = json_string_values(value);
    if values.is_empty() {
        return;
    }

    let rules = values
        .into_iter()
        .flat_map(|value| value.split(',').map(str::trim).map(str::to_owned).collect::<Vec<_>>())
        .filter(|value| !value.is_empty())
        .map(|value| format!("NETWORK,{value},{target}"))
        .collect::<Vec<_>>();
    if !rules.is_empty() {
        matchers.push(rules);
    }
}

fn push_json_ports(matchers: &mut Vec<Vec<String>>, value: Option<&JsonValue>, rule_type: &str, target: &str) {
    let values = json_string_values(value);
    if values.is_empty() {
        return;
    }

    let rules = values
        .into_iter()
        .flat_map(|value| value.split(',').map(str::trim).map(str::to_owned).collect::<Vec<_>>())
        .filter(|value| !value.is_empty())
        .map(|value| format!("{rule_type},{value},{target}"))
        .collect::<Vec<_>>();
    if !rules.is_empty() {
        matchers.push(rules);
    }
}

fn push_json_private_ip_rules(
    matchers: &mut Vec<Vec<String>>,
    value: Option<&JsonValue>,
    rule_type: &str,
    target: &str,
) {
    let is_private = match value {
        Some(JsonValue::Bool(value)) => *value,
        Some(JsonValue::String(value)) => value.eq_ignore_ascii_case("true"),
        _ => false,
    };

    if !is_private {
        return;
    }

    let ranges = ["10.0.0.0/8", "172.16.0.0/12", "192.168.0.0/16", "fc00::/7"];

    matchers.push(
        ranges
            .into_iter()
            .map(|value| format!("{rule_type},{value},{target}"))
            .collect(),
    );
}

fn parse_xray_domain_rule(value: &str, target: &str) -> Option<String> {
    let (rule_type, payload) = if let Some(value) = value.strip_prefix("regexp:") {
        ("DOMAIN-REGEX", value)
    } else if let Some(value) = value.strip_prefix("keyword:") {
        ("DOMAIN-KEYWORD", value)
    } else if let Some(value) = value.strip_prefix("full:") {
        ("DOMAIN", value)
    } else if let Some(value) = value.strip_prefix("domain:") {
        ("DOMAIN-SUFFIX", value)
    } else if let Some(value) = value.strip_prefix("geosite:") {
        ("GEOSITE", value)
    } else {
        return None;
    };

    Some(format!("{rule_type},{payload},{target}"))
}

fn parse_xray_ip_rule(value: &str, target: &str) -> Option<String> {
    if value.starts_with('!') || value.starts_with("ext:") {
        return None;
    }

    if let Some(value) = value.strip_prefix("geoip:") {
        return Some(format!("GEOIP,{value},{target}"));
    }

    if let Some(value) = exact_ip_cidr(value) {
        let rule_type = if value.contains(':') { "IP-CIDR6" } else { "IP-CIDR" };
        return Some(format!("{rule_type},{value},{target}"));
    }

    if value.contains('/') {
        let rule_type = if value.contains(':') { "IP-CIDR6" } else { "IP-CIDR" };
        return Some(format!("{rule_type},{value},{target}"));
    }

    None
}

fn exact_ip_cidr(value: &str) -> Option<String> {
    if is_ipv4(value) {
        Some(format!("{value}/32"))
    } else if is_ipv6(value) {
        Some(format!("{value}/128"))
    } else {
        None
    }
}

fn build_group_from_parts(
    name: &str,
    group_type: &str,
    positional: &[String],
    keyvals: &HashMap<String, String>,
) -> Option<JsonMap<String, JsonValue>> {
    let mut members = positional
        .iter()
        .map(|value| normalize_policy_name(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();

    if members.is_empty()
        && group_type != "relay"
        && !keyvals.contains_key("filter")
        && !keyvals.contains_key("use")
        && !keyvals.contains_key("policy-path")
    {
        return None;
    }

    if group_type != "relay" && !members.is_empty() && !members.iter().any(|member| member == "DIRECT") {
        members.push("DIRECT".into());
    }

    let mut group = JsonMap::new();
    group.insert("name".into(), JsonValue::String(name.to_owned()));
    group.insert("type".into(), JsonValue::String(group_type.to_owned()));
    if !members.is_empty() {
        group.insert(
            "proxies".into(),
            JsonValue::Array(members.into_iter().map(JsonValue::String).collect()),
        );
    }
    if let Some(url) = keyvals.get("url").filter(|value| !value.is_empty()) {
        group.insert("url".into(), JsonValue::String(strip_wrapping_quotes(url)));
    }
    if let Some(interval) = keyvals
        .get("interval")
        .and_then(|value| parse_duration_like(value).or_else(|| parse_integer(Some(value.as_str()))))
    {
        group.insert("interval".into(), JsonValue::Number(interval.into()));
    }
    if let Some(timeout) = keyvals
        .get("timeout")
        .and_then(|value| parse_duration_like(value).or_else(|| parse_integer(Some(value.as_str()))))
    {
        group.insert("timeout".into(), JsonValue::Number(timeout.into()));
    }
    if let Some(max_failed) = keyvals
        .get("max-failed-times")
        .and_then(|value| parse_integer(Some(value.as_str())))
    {
        group.insert("max-failed-times".into(), JsonValue::Number(max_failed.into()));
    }
    if let Some(filter) = keyvals
        .get("filter")
        .or_else(|| keyvals.get("server-tag-regex"))
        .or_else(|| keyvals.get("policy-regex"))
        .or_else(|| keyvals.get("resource-tag-regex"))
        .filter(|value| !value.is_empty())
    {
        group.insert("filter".into(), JsonValue::String(strip_wrapping_quotes(filter)));
    }
    if let Some(hidden) = keyvals.get("hidden").filter(|value| boolish_value(Some(value))) {
        let _ = hidden;
        group.insert("hidden".into(), JsonValue::Bool(true));
    }
    if let Some(interface_name) = keyvals
        .get("interface-name")
        .or_else(|| keyvals.get("interface"))
        .filter(|value| !value.is_empty())
    {
        group.insert(
            "interface-name".into(),
            JsonValue::String(strip_wrapping_quotes(interface_name)),
        );
    }
    if let Some(routing_mark) = keyvals
        .get("routing-mark")
        .or_else(|| keyvals.get("routing_mark"))
        .and_then(|value| parse_integer(Some(value.as_str())))
    {
        group.insert("routing-mark".into(), JsonValue::Number(routing_mark.into()));
    }
    if keyvals
        .get("disable-udp")
        .is_some_and(|value| boolish_value(Some(value)))
    {
        group.insert("disable-udp".into(), JsonValue::Bool(true));
    }
    if keyvals
        .get("include-all")
        .is_some_and(|value| boolish_value(Some(value)))
    {
        group.insert("include-all".into(), JsonValue::Bool(true));
    }
    if keyvals
        .get("include-all-proxies")
        .is_some_and(|value| boolish_value(Some(value)))
    {
        group.insert("include-all-proxies".into(), JsonValue::Bool(true));
    }
    if keyvals
        .get("include-all-providers")
        .is_some_and(|value| boolish_value(Some(value)))
    {
        group.insert("include-all-providers".into(), JsonValue::Bool(true));
    }
    if let Some(use_groups) = keyvals
        .get("include-other-group")
        .or_else(|| keyvals.get("use"))
        .filter(|value| !value.is_empty())
    {
        let values = split_csv(use_groups)
            .into_iter()
            .map(|value| normalize_policy_name(&value))
            .collect::<Vec<_>>();
        if !values.is_empty() {
            group.insert(
                "use".into(),
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
            );
        }
    }
    if let Some(lazy) = keyvals
        .get("lazy")
        .or_else(|| keyvals.get("alive-checking"))
        .filter(|value| !value.is_empty())
    {
        group.insert("lazy".into(), JsonValue::Bool(boolish_value(Some(lazy))));
    }
    if let Some(expected_status) = keyvals
        .get("expected-status")
        .or_else(|| keyvals.get("tolerance"))
        .filter(|value| !value.is_empty())
    {
        group.insert(
            if keyvals.contains_key("expected-status") {
                "expected-status".into()
            } else {
                "tolerance".into()
            },
            JsonValue::String(strip_wrapping_quotes(expected_status)),
        );
    }

    Some(group)
}

fn parse_rule_line(line: &str) -> Option<(String, Option<(String, JsonMap<String, JsonValue>)>, Vec<String>)> {
    let tokens = split_rule_like(line);
    if tokens.len() < 2 {
        return None;
    }

    let rule_type = tokens[0].trim().to_ascii_uppercase();
    if rule_type == "FINAL" {
        return Some((
            format!("MATCH,{}", normalize_policy_name(tokens[1].trim())),
            None,
            Vec::new(),
        ));
    }

    if tokens.len() < 3 {
        return None;
    }

    if rule_type == "RULE-SET" {
        let source = strip_wrapping_quotes(tokens[1].trim());
        let target = normalize_policy_name(tokens[2].trim());
        if target.is_empty() {
            return None;
        }

        let interval = tokens
            .iter()
            .skip(3)
            .find_map(|token| {
                token
                    .strip_prefix("interval=")
                    .or_else(|| token.strip_prefix("update-interval="))
            })
            .and_then(parse_duration_like);
        let provider_name = provider_name_from_source(&source);
        let provider = build_rule_provider_entry(&provider_name, &source, interval);
        let mut rule = format!("RULE-SET,{provider_name},{target}");
        if tokens
            .iter()
            .skip(3)
            .any(|token| token.eq_ignore_ascii_case("no-resolve"))
        {
            rule.push_str(",no-resolve");
        }
        let mut warnings = Vec::new();
        if tokens
            .iter()
            .skip(3)
            .any(|token| token.eq_ignore_ascii_case("extended-matching"))
        {
            warnings.push("Surge extended-matching on RULE-SET has no Mihomo equivalent and was ignored".into());
        }
        return Some((rule, Some((provider_name, provider)), warnings));
    }

    let mapped_type = normalize_rule_type(&rule_type)?;
    let target = normalize_policy_name(tokens[2].trim());
    let mut parts = vec![mapped_type.to_owned(), tokens[1].trim().to_owned(), target];
    if tokens
        .iter()
        .skip(3)
        .any(|token| token.eq_ignore_ascii_case("no-resolve"))
    {
        parts.push("no-resolve".into());
    }
    Some((parts.join(","), None, Vec::new()))
}

fn parse_qx_rule_line(line: &str) -> Option<String> {
    let tokens = split_rule_like(line);
    if tokens.len() < 2 {
        return None;
    }

    let rule_type = tokens[0].trim().to_ascii_lowercase();
    if rule_type == "final" {
        return Some(format!("MATCH,{}", normalize_policy_name(tokens[1].trim())));
    }

    if tokens.len() < 3 {
        return None;
    }

    let mapped_type = match rule_type.as_str() {
        "host" => "DOMAIN",
        "host-suffix" => "DOMAIN-SUFFIX",
        "host-keyword" => "DOMAIN-KEYWORD",
        "host-regex" => "DOMAIN-REGEX",
        "ip-cidr" => "IP-CIDR",
        "ip6-cidr" => "IP-CIDR6",
        "geoip" => "GEOIP",
        "ip-asn" => "IP-ASN",
        _ => return None,
    };

    let target = normalize_policy_name(tokens[2].trim());
    let mut parts = vec![mapped_type.into(), tokens[1].trim().to_owned(), target];
    if tokens
        .iter()
        .skip(3)
        .any(|token| token.eq_ignore_ascii_case("no-resolve"))
    {
        parts.push("no-resolve".into());
    }
    Some(parts.join(","))
}

fn build_rule_provider_entry(name: &str, source: &str, interval: Option<u64>) -> JsonMap<String, JsonValue> {
    let mut provider = JsonMap::new();
    provider.insert("behavior".into(), JsonValue::String("classical".into()));
    provider.insert(
        "format".into(),
        JsonValue::String(infer_rule_provider_format(source).into()),
    );
    provider.insert(
        "path".into(),
        JsonValue::String(format!("rulesets/{}.list", sanitize_provider_name(name))),
    );
    if let Some(interval) = interval {
        provider.insert("interval".into(), JsonValue::Number(interval.into()));
    }

    if looks_like_url(source) {
        provider.insert("type".into(), JsonValue::String("http".into()));
        provider.insert("url".into(), JsonValue::String(source.to_owned()));
    } else {
        provider.insert("type".into(), JsonValue::String("file".into()));
        provider.insert("path".into(), JsonValue::String(source.to_owned()));
    }

    provider
}

fn build_proxy_provider_entry(name: &str, source: &str, interval: Option<u64>) -> JsonMap<String, JsonValue> {
    let mut provider = JsonMap::new();
    provider.insert(
        "path".into(),
        JsonValue::String(format!("providers/{}.yaml", sanitize_provider_name(name))),
    );
    if let Some(interval) = interval {
        provider.insert("interval".into(), JsonValue::Number(interval.into()));
    }
    if looks_like_url(source) {
        provider.insert("type".into(), JsonValue::String("http".into()));
        provider.insert("url".into(), JsonValue::String(source.to_owned()));
    } else {
        provider.insert("type".into(), JsonValue::String("file".into()));
        provider.insert("path".into(), JsonValue::String(source.to_owned()));
    }
    provider
}

fn provider_name_from_source(source: &str) -> String {
    let last = source
        .rsplit('/')
        .next()
        .unwrap_or(source)
        .split('?')
        .next()
        .unwrap_or(source);
    let candidate = last.split('.').next().unwrap_or(last).trim();
    let candidate = if candidate.is_empty() { "ruleset" } else { candidate };
    sanitize_provider_name(candidate)
}

fn sanitize_provider_name(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_owned();
    if sanitized.is_empty() {
        "ruleset".into()
    } else {
        sanitized
    }
}

fn infer_rule_provider_format(source: &str) -> &'static str {
    let lower = source.to_ascii_lowercase();
    if lower.ends_with(".yaml") || lower.ends_with(".yml") {
        "yaml"
    } else if lower.ends_with(".mrs") || lower.ends_with(".srs") {
        "mrs"
    } else {
        "text"
    }
}

fn collect_surge_group_warnings(line: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    if line.contains("extended-matching") {
        warnings.push("Surge extended-matching is not supported and was ignored".into());
    }

    let Some((_, rhs_raw)) = line.split_once('=') else {
        return warnings;
    };
    let tokens = split_csv_like(rhs_raw);
    if tokens.len() < 2 {
        return warnings;
    }

    let (_, keyvals) = split_tokens(&tokens[1..]);
    let mut external_policy_keys = keyvals
        .keys()
        .filter(|key| key.starts_with("external-policy"))
        .cloned()
        .collect::<Vec<_>>();
    external_policy_keys.sort();

    for key in external_policy_keys {
        match key.as_str() {
            "external-policy-modifier" => {
                warnings.push("Surge external-policy-modifier has no Mihomo equivalent and was ignored".into());
            }
            "external-policy-name-prefix" => {
                warnings.push("Surge external-policy-name-prefix was not applied during translation".into());
            }
            other => warnings.push(format!("Surge `{other}` has no Mihomo equivalent and was ignored")),
        }
    }
    warnings
}

fn collect_sing_box_route_warnings(route: &serde_json::Map<String, JsonValue>) -> Vec<String> {
    let Some(rules) = route.get("rules").and_then(JsonValue::as_array) else {
        return Vec::new();
    };

    let mut warnings = Vec::new();
    for rule in rules.iter().filter_map(JsonValue::as_object) {
        if sing_box_rule_target(rule).is_none() {
            if let Some(action) = rule.get("action").and_then(JsonValue::as_str) {
                warnings.push(format!(
                    "sing-box route action `{action}` has no direct Mihomo equivalent and was ignored"
                ));
            }
            if sing_box_route_has_supported_matchers(rule) {
                warnings.push("sing-box route rule without a supported `outbound`/`action` target was ignored".into());
            }
        }

        for key in [
            "protocol",
            "client",
            "auth_user",
            "ip_version",
            "query_type",
            "wifi_ssid",
            "wifi_bssid",
            "network_type",
            "network_is_expensive",
            "network_is_constrained",
            "interface_address",
            "network_interface_address",
            "default_interface_address",
            "source_mac_address",
            "source_hostname",
            "preferred_by",
            "rule_set_ipcidr_match_source",
            "rule_set_ip_cidr_match_source",
            "rule_set_ip_cidr_accept_empty",
        ] {
            if rule.contains_key(key) {
                warnings.push(format!(
                    "sing-box route field `{key}` has no direct Mihomo equivalent and was ignored"
                ));
            }
        }
    }
    warnings
}

fn collect_xray_routing_warnings(routing: &serde_json::Map<String, JsonValue>) -> Vec<String> {
    let Some(rules) = routing.get("rules").and_then(JsonValue::as_array) else {
        return Vec::new();
    };

    let mut warnings = Vec::new();
    for rule in rules.iter().filter_map(JsonValue::as_object) {
        let has_target = rule.get("outboundTag").and_then(JsonValue::as_str).is_some()
            || rule.get("balancerTag").and_then(JsonValue::as_str).is_some();
        if !has_target && xray_rule_has_supported_matchers(rule) {
            warnings.push("Xray/V2Ray routing rule without `outboundTag` or `balancerTag` was ignored".into());
        }

        for key in [
            "attrs",
            "webhook",
            "ruleTag",
            "domainStrategy",
            "domainMatcher",
            "vlessRoute",
        ] {
            if rule.contains_key(key) {
                warnings.push(format!(
                    "Xray/V2Ray routing field `{key}` has no direct Mihomo equivalent and was ignored"
                ));
            }
        }

        for value in json_string_values(rule.get("domain")) {
            if !is_supported_xray_domain_value(&value) {
                warnings.push(format!(
                    "Xray/V2Ray domain matcher `{value}` has no direct Mihomo equivalent and was ignored"
                ));
            }
        }

        for value in json_string_values(rule.get("ip")) {
            if !is_supported_xray_ip_value(&value) {
                warnings.push(format!(
                    "Xray/V2Ray IP matcher `{value}` has no direct Mihomo equivalent and was ignored"
                ));
            }
        }

        for value in json_string_values(
            rule.get("sourceIP")
                .or_else(|| rule.get("source"))
                .or_else(|| rule.get("source_ip")),
        ) {
            if !is_supported_xray_ip_value(&value) {
                warnings.push(format!(
                    "Xray/V2Ray source matcher `{value}` has no direct Mihomo equivalent and was ignored"
                ));
            }
        }
    }
    warnings
}

fn sing_box_route_has_supported_matchers(rule: &serde_json::Map<String, JsonValue>) -> bool {
    !parse_common_json_rules(rule, "__mihomo__").is_empty()
}

fn xray_rule_has_supported_matchers(rule: &serde_json::Map<String, JsonValue>) -> bool {
    [
        "domain",
        "ip",
        "network",
        "port",
        "localPort",
        "local_port",
        "process",
        "sourcePort",
        "source_port",
        "sourceIP",
        "source",
        "source_ip",
        "user",
        "protocol",
        "inboundTag",
    ]
    .iter()
    .any(|key| rule.contains_key(*key))
}

fn is_supported_xray_domain_value(value: &str) -> bool {
    value.starts_with("regexp:")
        || value.starts_with("keyword:")
        || value.starts_with("full:")
        || value.starts_with("domain:")
        || value.starts_with("geosite:")
}

fn is_supported_xray_ip_value(value: &str) -> bool {
    if value.starts_with('!') || value.starts_with("ext:") {
        return false;
    }

    value.starts_with("geoip:") || value.contains('/') || exact_ip_cidr(value).is_some()
}

fn dedupe_warnings(warnings: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::new();
    for warning in warnings {
        if seen.insert(warning.clone()) {
            deduped.push(warning);
        }
    }
    deduped
}

fn collapse_matchers(matchers: Vec<Vec<String>>, target: &str, invert: bool) -> Vec<String> {
    if matchers.is_empty() {
        return Vec::new();
    }

    let mut group_bodies = Vec::new();
    for matcher_group in matchers {
        let mut bodies = Vec::new();
        for rule in matcher_group {
            let Some(body) = rule_to_expression_body(&rule, target) else {
                return Vec::new();
            };
            bodies.push(body);
        }
        if bodies.is_empty() {
            continue;
        }
        let group_body = if bodies.len() == 1 {
            bodies.remove(0)
        } else {
            format!(
                "OR,({})",
                bodies
                    .into_iter()
                    .map(|body| format!("({body})"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        };
        group_bodies.push(group_body);
    }

    if group_bodies.is_empty() {
        return Vec::new();
    }

    let combined_body = if group_bodies.len() == 1 {
        group_bodies.remove(0)
    } else {
        format!(
            "AND,({})",
            group_bodies
                .into_iter()
                .map(|body| format!("({body})"))
                .collect::<Vec<_>>()
                .join(",")
        )
    };

    if invert {
        vec![format!("NOT,(({combined_body})),{target}")]
    } else {
        vec![format!("{combined_body},{target}")]
    }
}

fn rule_to_expression_body(rule: &str, target: &str) -> Option<String> {
    let suffix = format!(",{target}");
    let suffix_no_resolve = format!(",{target},no-resolve");

    let body = if let Some(value) = rule.strip_suffix(&suffix_no_resolve) {
        format!("{value},no-resolve")
    } else if let Some(value) = rule.strip_suffix(&suffix) {
        value.to_owned()
    } else {
        return None;
    };

    Some(body)
}

fn json_object_to_map(value: JsonValue) -> Result<JsonMap<String, JsonValue>> {
    match value {
        JsonValue::Object(map) => Ok(map),
        _ => bail!("expected JSON object"),
    }
}

fn normalize_group_type(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "select" => Some("select"),
        "url-test" | "urltest" => Some("url-test"),
        "fallback" | "available" => Some("fallback"),
        "load-balance" | "loadbalance" | "round-robin" | "roundrobin" => Some("load-balance"),
        "relay" => Some("relay"),
        _ => None,
    }
}

fn normalize_qx_group_type(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "select" | "static" => Some("select"),
        "url-test" | "urltest" | "url-latency-benchmark" => Some("url-test"),
        "fallback" | "available" => Some("fallback"),
        "load-balance" | "loadbalance" | "round-robin" => Some("load-balance"),
        "relay" => Some("relay"),
        _ => None,
    }
}

fn normalize_policy_name(value: &str) -> String {
    let trimmed = strip_wrapping_quotes(value).trim().trim_start_matches("[]").to_owned();
    match trimmed.to_ascii_uppercase().as_str() {
        "DIRECT" | "REJECT" | "REJECT-DROP" | "PASS" => trimmed.to_ascii_uppercase(),
        "FINAL" => "MATCH".into(),
        other => {
            if other == "REJECT-TINYGIF" {
                "REJECT".into()
            } else {
                trimmed
            }
        }
    }
}

fn split_rule_like(input: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut depth = 0_i32;

    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            '(' if !quoted => {
                depth += 1;
                current.push(ch);
            }
            ')' if !quoted && depth > 0 => {
                depth -= 1;
                current.push(ch);
            }
            ',' if !quoted && depth == 0 => {
                let value = current.trim();
                if !value.is_empty() {
                    values.push(value.to_owned());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let value = current.trim();
    if !value.is_empty() {
        values.push(value.to_owned());
    }

    values
}

fn parse_duration_like(value: &str) -> Option<u64> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Ok(number) = trimmed.parse::<u64>() {
        return Some(number);
    }

    let (number, unit) = trimmed.chars().partition::<String, _>(|ch| ch.is_ascii_digit());
    let number = number.parse::<u64>().ok()?;
    match unit.as_str() {
        "ms" => Some(number / 1000),
        "s" => Some(number),
        "m" => Some(number * 60),
        "h" => Some(number * 3600),
        _ => None,
    }
}

fn looks_like_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn json_string_values(value: Option<&JsonValue>) -> Vec<String> {
    match value {
        Some(JsonValue::String(text)) => vec![text.to_owned()],
        Some(JsonValue::Array(values)) => values.iter().filter_map(value_as_string).collect(),
        _ => Vec::new(),
    }
}

fn value_as_string_ref(value: &JsonValue) -> Option<&str> {
    match value {
        JsonValue::String(text) => Some(text.as_str()),
        _ => None,
    }
}

fn normalize_rule_type(value: &str) -> Option<&'static str> {
    match value {
        "DOMAIN" => Some("DOMAIN"),
        "DOMAIN-SUFFIX" => Some("DOMAIN-SUFFIX"),
        "DOMAIN-KEYWORD" => Some("DOMAIN-KEYWORD"),
        "DOMAIN-REGEX" => Some("DOMAIN-REGEX"),
        "GEOSITE" => Some("GEOSITE"),
        "GEOIP" => Some("GEOIP"),
        "IP-ASN" => Some("IP-ASN"),
        "IP-CIDR" => Some("IP-CIDR"),
        "IP-CIDR6" => Some("IP-CIDR6"),
        "SRC-IP-CIDR" => Some("SRC-IP-CIDR"),
        "IP-SUFFIX" => Some("IP-SUFFIX"),
        "SRC-IP-SUFFIX" => Some("SRC-IP-SUFFIX"),
        "SRC-PORT" => Some("SRC-PORT"),
        "DST-PORT" => Some("DST-PORT"),
        "IN-PORT" => Some("IN-PORT"),
        "PROCESS-NAME" => Some("PROCESS-NAME"),
        "PROCESS-PATH" => Some("PROCESS-PATH"),
        "PROCESS-NAME-REGEX" => Some("PROCESS-NAME-REGEX"),
        "PROCESS-PATH-REGEX" => Some("PROCESS-PATH-REGEX"),
        "NETWORK" => Some("NETWORK"),
        "UID" => Some("UID"),
        "IN-TYPE" => Some("IN-TYPE"),
        "IN-USER" => Some("IN-USER"),
        "IN-NAME" => Some("IN-NAME"),
        "RULE-SET" => Some("RULE-SET"),
        "AND" => Some("AND"),
        "OR" => Some("OR"),
        "NOT" => Some("NOT"),
        "MATCH" => Some("MATCH"),
        _ => None,
    }
}

fn parse_ini_sections(input: &str) -> HashMap<String, Vec<String>> {
    let mut sections: HashMap<String, Vec<String>> = HashMap::new();
    let mut current_section = String::new();

    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() || is_comment_line(line) {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') && line.len() > 2 {
            current_section = line[1..line.len() - 1].trim().to_ascii_lowercase();
            continue;
        }

        sections
            .entry(current_section.clone())
            .or_default()
            .push(line.to_owned());
    }

    sections
}

fn parse_named_or_typed_proxy_line(line: &str) -> Result<Option<JsonMap<String, JsonValue>>> {
    let Some((left_raw, right_raw)) = line.split_once('=') else {
        return Ok(None);
    };

    let left = left_raw.trim();
    let right = right_raw.trim();
    if left.is_empty() || right.is_empty() {
        return Ok(None);
    }

    let tokens = split_csv_like(right);
    if tokens.is_empty() {
        return Ok(None);
    }

    let head_type = normalize_proxy_type(tokens[0].trim());
    if let Some(proxy_type) = head_type {
        return parse_surge_like_proxy(left.to_owned(), proxy_type, &tokens[1..]).map(Some);
    }

    if let Some(proxy_type) = normalize_proxy_type(left) {
        return parse_quantumult_like_proxy(proxy_type, &tokens).map(Some);
    }

    Ok(None)
}

fn parse_surge_like_proxy(name: String, proxy_type: String, tokens: &[String]) -> Result<JsonMap<String, JsonValue>> {
    let (positional, keyvals) = split_tokens(tokens);
    let (server, port, offset) = parse_server_port(&positional)?;
    build_proxy_from_tokens(name, &proxy_type, &server, port, &positional[offset..], &keyvals)
}

fn parse_quantumult_like_proxy(proxy_type: String, tokens: &[String]) -> Result<JsonMap<String, JsonValue>> {
    let (positional, keyvals) = split_tokens(tokens);
    let name = keyvals
        .get("tag")
        .cloned()
        .unwrap_or_else(|| proxy_type.to_ascii_uppercase());

    let (server, port, offset) = if let Some(first) = positional.first() {
        if let Some((host, port)) = split_host_port(first) {
            (host, port, 1)
        } else {
            let (server, port, offset) = parse_server_port(&positional)?;
            (server.to_owned(), port, offset)
        }
    } else {
        bail!("missing server in external config");
    };

    build_proxy_from_tokens(name, &proxy_type, &server, port, &positional[offset..], &keyvals)
}

fn build_proxy_from_tokens(
    name: String,
    proxy_type: &str,
    server: &str,
    port: u16,
    extra_positional: &[String],
    keyvals: &HashMap<String, String>,
) -> Result<JsonMap<String, JsonValue>> {
    let mut proxy = match proxy_type {
        "ss" => {
            let mut proxy = base_proxy_map("ss", name, server, port);
            let method = keyvals
                .get("encrypt-method")
                .or_else(|| keyvals.get("method"))
                .or_else(|| extra_positional.first())
                .map(String::as_str)
                .unwrap_or("auto");
            proxy.insert("cipher".into(), JsonValue::String(normalize_cipher(method)));
            if let Some(password) = keyvals.get("password").or_else(|| extra_positional.get(1)) {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            proxy
        }
        "ssr" => {
            let mut proxy = base_proxy_map("ssr", name, server, port);
            let method = keyvals
                .get("encrypt-method")
                .or_else(|| keyvals.get("method"))
                .or_else(|| extra_positional.first())
                .map(String::as_str)
                .unwrap_or("auto");
            proxy.insert("cipher".into(), JsonValue::String(normalize_cipher(method)));
            if let Some(password) = keyvals.get("password").or_else(|| extra_positional.get(1)) {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            insert_string_if_present(
                &mut proxy,
                "obfs",
                keyvals.get("obfs").or_else(|| extra_positional.get(2)),
            );
            insert_string_if_present(
                &mut proxy,
                "obfs-param",
                keyvals
                    .get("obfs-param")
                    .or_else(|| keyvals.get("obfsparam"))
                    .or_else(|| keyvals.get("obfs-host")),
            );
            insert_string_if_present(
                &mut proxy,
                "protocol",
                keyvals.get("protocol").or_else(|| extra_positional.get(3)),
            );
            insert_string_if_present(
                &mut proxy,
                "protocol-param",
                keyvals
                    .get("protocol-param")
                    .or_else(|| keyvals.get("protocol_param"))
                    .or_else(|| keyvals.get("protoparam")),
            );
            proxy
        }
        "vmess" => {
            let mut proxy = base_proxy_map("vmess", name, server, port);
            let method = keyvals
                .get("encrypt-method")
                .or_else(|| keyvals.get("method"))
                .or_else(|| extra_positional.first())
                .map(String::as_str)
                .unwrap_or("auto");
            proxy.insert("cipher".into(), JsonValue::String(normalize_cipher(method)));
            if let Some(uuid) = keyvals
                .get("username")
                .or_else(|| keyvals.get("uuid"))
                .or_else(|| keyvals.get("password"))
                .or_else(|| extra_positional.get(1))
            {
                proxy.insert("uuid".into(), JsonValue::String(strip_wrapping_quotes(uuid)));
            }
            if let Some(alter_id) = keyvals
                .get("alterid")
                .or_else(|| keyvals.get("alter-id"))
                .and_then(|value| parse_integer(Some(value.as_str())))
            {
                proxy.insert("alterId".into(), JsonValue::Number(alter_id.into()));
            }
            proxy
        }
        "vless" => {
            let mut proxy = base_proxy_map("vless", name, server, port);
            if let Some(uuid) = keyvals
                .get("username")
                .or_else(|| keyvals.get("uuid"))
                .or_else(|| keyvals.get("password"))
                .or_else(|| extra_positional.first())
            {
                proxy.insert("uuid".into(), JsonValue::String(strip_wrapping_quotes(uuid)));
            }
            if let Some(flow) = keyvals.get("flow").filter(|value| is_valid_vless_flow(value)) {
                proxy.insert("flow".into(), JsonValue::String(flow.clone()));
            }
            proxy
        }
        "trojan" => {
            let mut proxy = base_proxy_map("trojan", name, server, port);
            if let Some(password) = keyvals.get("password").or_else(|| extra_positional.first()) {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            proxy
        }
        "ssh" => {
            let mut proxy = base_proxy_map("ssh", name, server, port);
            if let Some(username) = keyvals.get("username").or_else(|| extra_positional.first()) {
                proxy.insert("username".into(), JsonValue::String(strip_wrapping_quotes(username)));
            }
            if let Some(password) = keyvals.get("password").or_else(|| extra_positional.get(1)) {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            insert_string_if_present(
                &mut proxy,
                "private-key",
                keyvals.get("private-key").or_else(|| keyvals.get("private_key")),
            );
            insert_string_if_present(
                &mut proxy,
                "private-key-passphrase",
                keyvals
                    .get("private-key-passphrase")
                    .or_else(|| keyvals.get("private_key_passphrase")),
            );
            insert_string_if_present(
                &mut proxy,
                "host-key",
                keyvals.get("host-key").or_else(|| keyvals.get("host_key")),
            );
            insert_string_if_present(
                &mut proxy,
                "host-key-algorithms",
                keyvals
                    .get("host-key-algorithms")
                    .or_else(|| keyvals.get("host_key_algorithms")),
            );
            proxy
        }
        "anytls" => {
            let mut proxy = base_proxy_map("anytls", name, server, port);
            if let Some(password) = keyvals.get("password").or_else(|| extra_positional.first()) {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            proxy.insert("udp".into(), JsonValue::Bool(true));
            proxy
        }
        "snell" => {
            let mut proxy = base_proxy_map("snell", name, server, port);
            if let Some(psk) = keyvals
                .get("psk")
                .or_else(|| keyvals.get("password"))
                .or_else(|| extra_positional.first())
            {
                proxy.insert("psk".into(), JsonValue::String(strip_wrapping_quotes(psk)));
            }
            if let Some(version) = keyvals
                .get("version")
                .and_then(|value| parse_integer(Some(value.as_str())))
            {
                proxy.insert("version".into(), JsonValue::Number(version.into()));
            }
            if keyvals.get("udp").is_some_and(|value| boolish_value(Some(value))) {
                proxy.insert("udp".into(), JsonValue::Bool(true));
            }
            proxy
        }
        "mieru" => {
            let mut proxy = base_proxy_map("mieru", name, server, port);
            insert_string_if_present(
                &mut proxy,
                "port-range",
                keyvals.get("port-range").or_else(|| keyvals.get("port_range")),
            );
            insert_string_if_present(&mut proxy, "transport", keyvals.get("transport"));
            insert_string_if_present(&mut proxy, "username", keyvals.get("username"));
            insert_string_if_present(&mut proxy, "password", keyvals.get("password"));
            insert_string_if_present(&mut proxy, "multiplexing", keyvals.get("multiplexing"));
            insert_string_if_present(
                &mut proxy,
                "handshake-mode",
                keyvals.get("handshake-mode").or_else(|| keyvals.get("handshake_mode")),
            );
            if keyvals.get("udp").is_some_and(|value| boolish_value(Some(value))) {
                proxy.insert("udp".into(), JsonValue::Bool(true));
            }
            proxy
        }
        "masque" => {
            let mut proxy = base_proxy_map("masque", name, server, port);
            insert_string_if_present(
                &mut proxy,
                "private-key",
                keyvals.get("private-key").or_else(|| keyvals.get("private_key")),
            );
            insert_string_if_present(
                &mut proxy,
                "public-key",
                keyvals.get("public-key").or_else(|| keyvals.get("public_key")),
            );
            insert_string_if_present(&mut proxy, "ip", keyvals.get("ip"));
            insert_string_if_present(&mut proxy, "ipv6", keyvals.get("ipv6"));
            if let Some(mtu) = keyvals.get("mtu").and_then(|value| parse_integer(Some(value.as_str()))) {
                proxy.insert("mtu".into(), JsonValue::Number(mtu.into()));
            }
            if keyvals.get("udp").is_some_and(|value| boolish_value(Some(value))) {
                proxy.insert("udp".into(), JsonValue::Bool(true));
            }
            if keyvals
                .get("remote-dns-resolve")
                .is_some_and(|value| boolish_value(Some(value)))
            {
                proxy.insert("remote-dns-resolve".into(), JsonValue::Bool(true));
            }
            if let Some(dns) = keyvals.get("dns") {
                let values = split_csv(dns);
                if !values.is_empty() {
                    proxy.insert(
                        "dns".into(),
                        JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
                    );
                }
            }
            proxy
        }
        "sudoku" => {
            let mut proxy = base_proxy_map("sudoku", name, server, port);
            insert_string_if_present(&mut proxy, "key", keyvals.get("key"));
            insert_string_if_present(
                &mut proxy,
                "aead-method",
                keyvals.get("aead-method").or_else(|| keyvals.get("aead_method")),
            );
            insert_string_if_present(
                &mut proxy,
                "table-type",
                keyvals.get("table-type").or_else(|| keyvals.get("table_type")),
            );
            insert_string_if_present(
                &mut proxy,
                "http-mask-mode",
                keyvals.get("http-mask-mode").or_else(|| keyvals.get("http_mask_mode")),
            );
            insert_string_if_present(
                &mut proxy,
                "http-mask-host",
                keyvals.get("http-mask-host").or_else(|| keyvals.get("http_mask_host")),
            );
            if let Some(min) = keyvals
                .get("padding-min")
                .or_else(|| keyvals.get("padding_min"))
                .and_then(|value| parse_integer(Some(value.as_str())))
            {
                proxy.insert("padding-min".into(), JsonValue::Number(min.into()));
            }
            if let Some(max) = keyvals
                .get("padding-max")
                .or_else(|| keyvals.get("padding_max"))
                .and_then(|value| parse_integer(Some(value.as_str())))
            {
                proxy.insert("padding-max".into(), JsonValue::Number(max.into()));
            }
            proxy
        }
        "http" => {
            let mut proxy = base_proxy_map("http", name, server, port);
            if let Some(username) = keyvals.get("username").or_else(|| extra_positional.first()) {
                proxy.insert("username".into(), JsonValue::String(strip_wrapping_quotes(username)));
            }
            if let Some(password) = keyvals.get("password").or_else(|| extra_positional.get(1)) {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            proxy
        }
        "socks5" => {
            let mut proxy = base_proxy_map("socks5", name, server, port);
            if let Some(username) = keyvals.get("username").or_else(|| extra_positional.first()) {
                proxy.insert("username".into(), JsonValue::String(strip_wrapping_quotes(username)));
            }
            if let Some(password) = keyvals.get("password").or_else(|| extra_positional.get(1)) {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            proxy
        }
        "hysteria" => {
            let mut proxy = base_proxy_map("hysteria", name, server, port);
            if let Some(auth) = keyvals
                .get("auth-str")
                .or_else(|| keyvals.get("auth"))
                .or_else(|| extra_positional.first())
            {
                proxy.insert("auth-str".into(), JsonValue::String(strip_wrapping_quotes(auth)));
            }
            insert_string_if_present(&mut proxy, "obfs", keyvals.get("obfs"));
            insert_string_if_present(
                &mut proxy,
                "ports",
                keyvals.get("ports").or_else(|| keyvals.get("mport")),
            );
            insert_string_if_present(
                &mut proxy,
                "protocol",
                keyvals.get("protocol").or_else(|| keyvals.get("obfs-protocol")),
            );
            insert_string_if_present(
                &mut proxy,
                "up",
                keyvals
                    .get("up")
                    .or_else(|| keyvals.get("upmbps"))
                    .or_else(|| keyvals.get("up-speed")),
            );
            insert_string_if_present(
                &mut proxy,
                "down",
                keyvals
                    .get("down")
                    .or_else(|| keyvals.get("downmbps"))
                    .or_else(|| keyvals.get("down-speed")),
            );
            proxy
        }
        "hysteria2" => {
            let mut proxy = base_proxy_map("hysteria2", name, server, port);
            if let Some(password) = keyvals
                .get("password")
                .or_else(|| keyvals.get("auth"))
                .or_else(|| extra_positional.first())
            {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            proxy
        }
        "tuic" => {
            let mut proxy = base_proxy_map("tuic", name, server, port);
            if let Some(uuid) = keyvals
                .get("uuid")
                .or_else(|| keyvals.get("username"))
                .or_else(|| extra_positional.first())
            {
                proxy.insert("uuid".into(), JsonValue::String(strip_wrapping_quotes(uuid)));
            }
            if let Some(password) = keyvals
                .get("password")
                .or_else(|| extra_positional.get(1))
                .or_else(|| keyvals.get("token"))
            {
                proxy.insert("password".into(), JsonValue::String(strip_wrapping_quotes(password)));
            }
            insert_string_if_present(&mut proxy, "token", keyvals.get("token"));
            proxy
        }
        "wireguard" => {
            let mut proxy = base_proxy_map("wireguard", name, server, port);
            insert_string_if_present(
                &mut proxy,
                "private-key",
                keyvals.get("private-key").or_else(|| keyvals.get("secret-key")),
            );
            insert_string_if_present(
                &mut proxy,
                "public-key",
                keyvals.get("public-key").or_else(|| keyvals.get("peer-public-key")),
            );
            insert_string_if_present(&mut proxy, "pre-shared-key", keyvals.get("pre-shared-key"));
            proxy.insert("udp".into(), JsonValue::Bool(true));
            proxy
        }
        _ => bail!("unsupported external proxy type: {proxy_type}"),
    };

    apply_common_proxy_options(&mut proxy, proxy_type, keyvals);
    Ok(proxy)
}

fn apply_common_proxy_options(
    proxy: &mut JsonMap<String, JsonValue>,
    proxy_type: &str,
    keyvals: &HashMap<String, String>,
) {
    if let Some(value) = keyvals.get("tls").or_else(|| keyvals.get("secure")) {
        proxy.insert("tls".into(), JsonValue::Bool(boolish_value(Some(value))));
    }
    if matches!(proxy_type, "http" | "socks5")
        && keyvals
            .get("type")
            .is_some_and(|value| value.eq_ignore_ascii_case("https"))
    {
        proxy.insert("tls".into(), JsonValue::Bool(true));
    }
    if keyvals
        .get("obfs")
        .is_some_and(|value| value.eq_ignore_ascii_case("wss"))
    {
        proxy.insert("tls".into(), JsonValue::Bool(true));
    }
    if let Some(sni) = keyvals
        .get("sni")
        .or_else(|| keyvals.get("servername"))
        .or_else(|| keyvals.get("peer"))
        .filter(|value| !value.is_empty())
    {
        let key = if matches!(proxy_type, "trojan" | "anytls" | "hysteria" | "hysteria2" | "tuic") {
            "sni"
        } else {
            "servername"
        };
        proxy.insert(key.into(), JsonValue::String(strip_wrapping_quotes(sni)));
    }
    if let Some(verify) = keyvals.get("tls-verification").and_then(|value| parse_bool(value)) {
        proxy.insert("skip-cert-verify".into(), JsonValue::Bool(!verify));
    } else if let Some(skip) = keyvals
        .get("skip-cert-verify")
        .or_else(|| keyvals.get("allow-insecure"))
        .map(|value| boolish_value(Some(value)))
    {
        proxy.insert("skip-cert-verify".into(), JsonValue::Bool(skip));
    }
    if let Some(alpn) = keyvals.get("alpn") {
        let values = split_csv(alpn);
        if !values.is_empty() {
            proxy.insert(
                "alpn".into(),
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
            );
        }
    }
    if let Some(fingerprint) = keyvals
        .get("client-fingerprint")
        .or_else(|| keyvals.get("fingerprint"))
        .filter(|value| !value.is_empty())
    {
        let key = if keyvals.contains_key("client-fingerprint") {
            "client-fingerprint"
        } else {
            "fingerprint"
        };
        proxy.insert(key.into(), JsonValue::String(strip_wrapping_quotes(fingerprint)));
    }
    if let Some(flow) = keyvals.get("flow").filter(|value| is_valid_vless_flow(value)) {
        proxy.insert("flow".into(), JsonValue::String(flow.clone()));
    }
    if keyvals.get("udp-relay").is_some_and(|value| boolish_value(Some(value)))
        || keyvals.get("udp").is_some_and(|value| boolish_value(Some(value)))
    {
        proxy.insert("udp".into(), JsonValue::Bool(true));
    }
    if keyvals.get("fast-open").is_some_and(|value| boolish_value(Some(value)))
        || keyvals.get("tfo").is_some_and(|value| boolish_value(Some(value)))
    {
        proxy.insert("tfo".into(), JsonValue::Bool(true));
    }
    if let Some(obfs) = keyvals.get("obfs").map(|value| value.to_ascii_lowercase()) {
        match obfs.as_str() {
            "ws" | "wss" => {
                proxy.insert("network".into(), JsonValue::String("ws".into()));
                let path = keyvals
                    .get("obfs-uri")
                    .or_else(|| keyvals.get("ws-path"))
                    .or_else(|| keyvals.get("path"))
                    .map(|value| strip_wrapping_quotes(value))
                    .filter(|value| !value.is_empty());
                let host = keyvals
                    .get("obfs-host")
                    .or_else(|| keyvals.get("ws-host"))
                    .or_else(|| keyvals.get("host"))
                    .map(|value| strip_wrapping_quotes(value))
                    .filter(|value| !value.is_empty());
                let mut opts = JsonMap::new();
                if let Some(path) = path {
                    opts.insert("path".into(), JsonValue::String(path));
                }
                if let Some(host) = host {
                    opts.insert("headers".into(), json!({ "Host": host }));
                }
                if !opts.is_empty() {
                    proxy.insert("ws-opts".into(), JsonValue::Object(opts));
                }
            }
            "grpc" => {
                proxy.insert("network".into(), JsonValue::String("grpc".into()));
                if let Some(service_name) = keyvals
                    .get("grpc-service-name")
                    .map(|value| strip_wrapping_quotes(value))
                    .filter(|value| !value.is_empty())
                {
                    proxy.insert("grpc-opts".into(), json!({ "grpc-service-name": service_name }));
                }
            }
            _ => {}
        }
    } else if keyvals.get("grpc").is_some_and(|value| boolish_value(Some(value))) {
        proxy.insert("network".into(), JsonValue::String("grpc".into()));
        if let Some(service_name) = keyvals
            .get("grpc-service-name")
            .map(|value| strip_wrapping_quotes(value))
            .filter(|value| !value.is_empty())
        {
            proxy.insert("grpc-opts".into(), json!({ "grpc-service-name": service_name }));
        }
    }
}

fn split_tokens(tokens: &[String]) -> (Vec<String>, HashMap<String, String>) {
    let mut positional = Vec::new();
    let mut keyvals = HashMap::new();

    for token in tokens {
        if let Some((key, value)) = token.split_once('=') {
            let key = key.trim().to_ascii_lowercase();
            if !key.is_empty() {
                keyvals.insert(key, strip_wrapping_quotes(value));
                continue;
            }
        }
        positional.push(strip_wrapping_quotes(token));
    }

    (positional, keyvals)
}

fn parse_server_port(positional: &[String]) -> Result<(String, u16, usize)> {
    let Some(server) = positional.first() else {
        bail!("missing server");
    };
    if let Some((host, port)) = split_host_port(server) {
        return Ok((host, port, 1));
    }

    let port = positional
        .get(1)
        .ok_or_else(|| anyhow::anyhow!("missing port"))
        .and_then(|value| parse_required_port(value, "invalid port"))?;
    Ok((server.to_owned(), port, 2))
}

fn split_host_port(input: &str) -> Option<(String, u16)> {
    let trimmed = input.trim();
    if trimmed.starts_with('[') {
        let end = trimmed.find(']')?;
        let host = trimmed[1..end].to_owned();
        let port = trimmed[end + 1..]
            .strip_prefix(':')
            .and_then(|value| value.parse::<u16>().ok())?;
        return Some((host, port));
    }

    let idx = trimmed.rfind(':')?;
    let host = trimmed[..idx].to_owned();
    let port = trimmed[idx + 1..].parse::<u16>().ok()?;
    Some((host, port))
}

fn normalize_proxy_type(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    let mapped = match normalized.as_str() {
        "shadowsocks" => "ss",
        "shadowsocksr" => "ssr",
        "https" => "http",
        "socks5-tls" => "socks5",
        "ssh-proxy" => "ssh",
        "snell-v3" => "snell",
        "hy" | "hy1" => "hysteria",
        "hy2" => "hysteria2",
        "socks" => "socks5",
        other => other,
    };

    matches!(
        mapped,
        "ss" | "ssr"
            | "vmess"
            | "vless"
            | "trojan"
            | "ssh"
            | "snell"
            | "anytls"
            | "mieru"
            | "masque"
            | "sudoku"
            | "http"
            | "socks5"
            | "hysteria"
            | "hysteria2"
            | "tuic"
            | "wireguard"
    )
    .then(|| mapped.to_owned())
}

fn split_csv_like(input: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut current = String::new();
    let mut quoted = false;

    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                current.push(ch);
            }
            ',' if !quoted => {
                let value = current.trim();
                if !value.is_empty() {
                    values.push(value.to_owned());
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    let value = current.trim();
    if !value.is_empty() {
        values.push(value.to_owned());
    }

    values
}

fn strip_wrapping_quotes(value: &str) -> String {
    value.trim().trim_matches('"').to_owned()
}

fn first_array_object(value: Option<&JsonValue>) -> Option<&serde_json::Map<String, JsonValue>> {
    value
        .and_then(JsonValue::as_array)
        .and_then(|items| items.first())
        .and_then(JsonValue::as_object)
}

fn value_as_u16(value: &JsonValue) -> Option<u16> {
    value_as_u64(value).and_then(|value| u16::try_from(value).ok())
}

fn value_as_u64(value: &JsonValue) -> Option<u64> {
    match value {
        JsonValue::Number(number) => number.as_u64(),
        JsonValue::String(text) => text.parse::<u64>().ok(),
        _ => None,
    }
}

fn insert_string_field(proxy: &mut JsonMap<String, JsonValue>, key: &str, value: Option<&JsonValue>) {
    if let Some(value) = value.and_then(value_as_string).filter(|value| !value.is_empty()) {
        proxy.insert(key.into(), JsonValue::String(value));
    }
}

fn insert_string_if_present(proxy: &mut JsonMap<String, JsonValue>, key: &str, value: Option<&String>) {
    if let Some(value) = value.filter(|value| !value.is_empty()) {
        proxy.insert(key.into(), JsonValue::String(strip_wrapping_quotes(value)));
    }
}

fn value_as_string(value: &JsonValue) -> Option<String> {
    match value {
        JsonValue::String(text) => Some(text.to_owned()),
        JsonValue::Number(number) => Some(number.to_string()),
        JsonValue::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}

fn apply_tls_object(proxy: &mut JsonMap<String, JsonValue>, tls: Option<&JsonValue>) {
    let Some(tls) = tls.and_then(JsonValue::as_object) else {
        return;
    };

    if !tls.get("enabled").and_then(JsonValue::as_bool).unwrap_or(false) {
        return;
    }

    proxy.insert("tls".into(), JsonValue::Bool(true));
    if let Some(server_name) = tls
        .get("server_name")
        .and_then(JsonValue::as_str)
        .filter(|value| !value.is_empty())
    {
        proxy.insert("servername".into(), JsonValue::String(server_name.to_owned()));
        proxy.insert("sni".into(), JsonValue::String(server_name.to_owned()));
    }
    if let Some(insecure) = tls.get("insecure").and_then(JsonValue::as_bool) {
        proxy.insert("skip-cert-verify".into(), JsonValue::Bool(insecure));
    }
    if let Some(alpn) = tls.get("alpn").and_then(JsonValue::as_array) {
        let values = alpn
            .iter()
            .filter_map(JsonValue::as_str)
            .map(std::borrow::ToOwned::to_owned)
            .collect::<Vec<_>>();
        if !values.is_empty() {
            proxy.insert(
                "alpn".into(),
                JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
            );
        }
    }
    if let Some(fingerprint) = tls
        .get("utls")
        .and_then(JsonValue::as_object)
        .and_then(|utls| utls.get("fingerprint"))
        .and_then(JsonValue::as_str)
    {
        proxy.insert("client-fingerprint".into(), JsonValue::String(fingerprint.to_owned()));
    }
    if let Some(reality) = tls.get("reality").and_then(JsonValue::as_object) {
        let mut reality_opts = JsonMap::new();
        if let Some(public_key) = reality.get("public_key").and_then(JsonValue::as_str) {
            reality_opts.insert("public-key".into(), JsonValue::String(public_key.to_owned()));
        }
        if let Some(short_id) = reality.get("short_id").and_then(JsonValue::as_str) {
            reality_opts.insert("short-id".into(), JsonValue::String(short_id.to_owned()));
        }
        if !reality_opts.is_empty() {
            proxy.insert("reality-opts".into(), JsonValue::Object(reality_opts));
        }
    }
}

fn apply_transport_object(proxy: &mut JsonMap<String, JsonValue>, transport: Option<&JsonValue>) {
    let Some(transport) = transport.and_then(JsonValue::as_object) else {
        return;
    };
    let Some(transport_type) = transport.get("type").and_then(JsonValue::as_str) else {
        return;
    };

    match transport_type {
        "ws" => {
            proxy.insert("network".into(), JsonValue::String("ws".into()));
            let mut opts = JsonMap::new();
            if let Some(path) = transport
                .get("path")
                .and_then(JsonValue::as_str)
                .filter(|value| !value.is_empty())
            {
                opts.insert("path".into(), JsonValue::String(path.to_owned()));
            }
            if let Some(host) = transport
                .get("headers")
                .and_then(JsonValue::as_object)
                .and_then(|headers| headers.get("Host"))
                .and_then(value_as_string)
            {
                opts.insert("headers".into(), json!({ "Host": host }));
            }
            if !opts.is_empty() {
                proxy.insert("ws-opts".into(), JsonValue::Object(opts));
            }
        }
        "grpc" => {
            proxy.insert("network".into(), JsonValue::String("grpc".into()));
            if let Some(service_name) = transport
                .get("service_name")
                .or_else(|| transport.get("serviceName"))
                .and_then(JsonValue::as_str)
                .filter(|value| !value.is_empty())
            {
                proxy.insert("grpc-opts".into(), json!({ "grpc-service-name": service_name }));
            }
        }
        "http" => {
            proxy.insert("network".into(), JsonValue::String("http".into()));
            let mut opts = JsonMap::new();
            if let Some(path) = transport.get("path").and_then(JsonValue::as_str) {
                opts.insert(
                    "path".into(),
                    JsonValue::Array(vec![JsonValue::String(path.to_owned())]),
                );
            }
            if let Some(hosts) = transport.get("host").and_then(JsonValue::as_array) {
                let host_values = hosts
                    .iter()
                    .filter_map(JsonValue::as_str)
                    .map(std::borrow::ToOwned::to_owned)
                    .collect::<Vec<_>>();
                if !host_values.is_empty() {
                    opts.insert("headers".into(), json!({ "Host": host_values }));
                }
            }
            if !opts.is_empty() {
                proxy.insert("http-opts".into(), JsonValue::Object(opts));
            }
        }
        "httpupgrade" => {
            proxy.insert("network".into(), JsonValue::String("ws".into()));
            let mut opts = JsonMap::new();
            if let Some(path) = transport.get("path").and_then(JsonValue::as_str) {
                opts.insert("path".into(), JsonValue::String(path.to_owned()));
            }
            if let Some(host) = transport.get("host").and_then(JsonValue::as_str) {
                opts.insert("headers".into(), json!({ "Host": host }));
            }
            opts.insert("v2ray-http-upgrade".into(), JsonValue::Bool(true));
            opts.insert("v2ray-http-upgrade-fast-open".into(), JsonValue::Bool(true));
            proxy.insert("ws-opts".into(), JsonValue::Object(opts));
        }
        _ => {}
    }
}

fn apply_xray_stream_settings(proxy: &mut JsonMap<String, JsonValue>, stream_settings: Option<&JsonValue>) {
    let Some(stream_settings) = stream_settings.and_then(JsonValue::as_object) else {
        return;
    };

    match stream_settings.get("security").and_then(JsonValue::as_str) {
        Some("tls") => {
            proxy.insert("tls".into(), JsonValue::Bool(true));
            if let Some(tls) = stream_settings.get("tlsSettings").and_then(JsonValue::as_object) {
                if let Some(server_name) = tls.get("serverName").and_then(JsonValue::as_str) {
                    proxy.insert("servername".into(), JsonValue::String(server_name.to_owned()));
                    proxy.insert("sni".into(), JsonValue::String(server_name.to_owned()));
                }
                if let Some(allow_insecure) = tls.get("allowInsecure").and_then(JsonValue::as_bool) {
                    proxy.insert("skip-cert-verify".into(), JsonValue::Bool(allow_insecure));
                }
                if let Some(alpn) = tls.get("alpn").and_then(JsonValue::as_array) {
                    let values = alpn
                        .iter()
                        .filter_map(JsonValue::as_str)
                        .map(std::borrow::ToOwned::to_owned)
                        .collect::<Vec<_>>();
                    if !values.is_empty() {
                        proxy.insert(
                            "alpn".into(),
                            JsonValue::Array(values.into_iter().map(JsonValue::String).collect()),
                        );
                    }
                }
                if let Some(fingerprint) = tls.get("fingerprint").and_then(JsonValue::as_str) {
                    proxy.insert("client-fingerprint".into(), JsonValue::String(fingerprint.to_owned()));
                }
            }
        }
        Some("reality") => {
            proxy.insert("tls".into(), JsonValue::Bool(true));
            if let Some(reality) = stream_settings.get("realitySettings").and_then(JsonValue::as_object) {
                if let Some(server_name) = reality.get("serverName").and_then(JsonValue::as_str) {
                    proxy.insert("servername".into(), JsonValue::String(server_name.to_owned()));
                    proxy.insert("sni".into(), JsonValue::String(server_name.to_owned()));
                }
                if let Some(fingerprint) = reality.get("fingerprint").and_then(JsonValue::as_str) {
                    proxy.insert("client-fingerprint".into(), JsonValue::String(fingerprint.to_owned()));
                }
                let mut reality_opts = JsonMap::new();
                if let Some(public_key) = reality.get("publicKey").and_then(JsonValue::as_str) {
                    reality_opts.insert("public-key".into(), JsonValue::String(public_key.to_owned()));
                }
                if let Some(short_id) = reality.get("shortId").and_then(JsonValue::as_str) {
                    reality_opts.insert("short-id".into(), JsonValue::String(short_id.to_owned()));
                }
                if !reality_opts.is_empty() {
                    proxy.insert("reality-opts".into(), JsonValue::Object(reality_opts));
                }
            }
        }
        _ => {}
    }

    match stream_settings.get("network").and_then(JsonValue::as_str) {
        Some("ws") => {
            proxy.insert("network".into(), JsonValue::String("ws".into()));
            let mut opts = JsonMap::new();
            if let Some(ws) = stream_settings.get("wsSettings").and_then(JsonValue::as_object) {
                if let Some(path) = ws.get("path").and_then(JsonValue::as_str) {
                    opts.insert("path".into(), JsonValue::String(path.to_owned()));
                }
                if let Some(host) = ws
                    .get("headers")
                    .and_then(JsonValue::as_object)
                    .and_then(|headers| headers.get("Host"))
                    .and_then(value_as_string)
                {
                    opts.insert("headers".into(), json!({ "Host": host }));
                }
            }
            if !opts.is_empty() {
                proxy.insert("ws-opts".into(), JsonValue::Object(opts));
            }
        }
        Some("grpc") => {
            proxy.insert("network".into(), JsonValue::String("grpc".into()));
            if let Some(service_name) = stream_settings
                .get("grpcSettings")
                .and_then(JsonValue::as_object)
                .and_then(|grpc| grpc.get("serviceName"))
                .and_then(JsonValue::as_str)
            {
                proxy.insert("grpc-opts".into(), json!({ "grpc-service-name": service_name }));
            }
        }
        Some("http") | Some("h2") => {
            let network = if stream_settings.get("network").and_then(JsonValue::as_str) == Some("h2") {
                "h2"
            } else {
                "http"
            };
            proxy.insert("network".into(), JsonValue::String(network.into()));
            if let Some(http) = stream_settings.get("httpSettings").and_then(JsonValue::as_object) {
                let mut opts = JsonMap::new();
                if let Some(path) = http.get("path").and_then(JsonValue::as_str) {
                    if network == "http" {
                        opts.insert(
                            "path".into(),
                            JsonValue::Array(vec![JsonValue::String(path.to_owned())]),
                        );
                    } else {
                        opts.insert("path".into(), JsonValue::String(path.to_owned()));
                    }
                }
                if let Some(hosts) = http.get("host").and_then(JsonValue::as_array) {
                    let values = hosts
                        .iter()
                        .filter_map(JsonValue::as_str)
                        .map(std::borrow::ToOwned::to_owned)
                        .collect::<Vec<_>>();
                    if !values.is_empty() {
                        if network == "http" {
                            opts.insert("headers".into(), json!({ "Host": values }));
                        } else if let Some(first) = values.first() {
                            opts.insert("host".into(), JsonValue::String(first.clone()));
                        }
                    }
                }
                if !opts.is_empty() {
                    let key = if network == "http" { "http-opts" } else { "h2-opts" };
                    proxy.insert(key.into(), JsonValue::Object(opts));
                }
            }
        }
        Some("httpupgrade") => {
            proxy.insert("network".into(), JsonValue::String("ws".into()));
            if let Some(httpupgrade) = stream_settings
                .get("httpupgradeSettings")
                .or_else(|| stream_settings.get("httpUpgradeSettings"))
                .and_then(JsonValue::as_object)
            {
                let mut opts = JsonMap::new();
                if let Some(path) = httpupgrade.get("path").and_then(JsonValue::as_str) {
                    opts.insert("path".into(), JsonValue::String(path.to_owned()));
                }
                if let Some(host) = httpupgrade.get("host").and_then(JsonValue::as_str) {
                    opts.insert("headers".into(), json!({ "Host": host }));
                }
                opts.insert("v2ray-http-upgrade".into(), JsonValue::Bool(true));
                opts.insert("v2ray-http-upgrade-fast-open".into(), JsonValue::Bool(true));
                proxy.insert("ws-opts".into(), JsonValue::Object(opts));
            }
        }
        _ => {}
    }
}

fn json_value_to_string(value: JsonValue) -> String {
    match value {
        JsonValue::Null => String::new(),
        JsonValue::Bool(value) => value.to_string(),
        JsonValue::Number(value) => value.to_string(),
        JsonValue::String(value) => value,
        other => other.to_string(),
    }
}

fn is_ipv4(address: &str) -> bool {
    let parts = address.split('.').collect::<Vec<_>>();
    parts.len() == 4 && parts.iter().all(|part| !part.is_empty() && part.parse::<u8>().is_ok())
}

fn is_ipv6(address: &str) -> bool {
    address.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_generated_yaml(input: &str) -> Mapping {
        serde_yaml_ng::from_str::<Mapping>(input).expect("generated yaml should parse")
    }

    #[test]
    fn normalizes_single_ss_uri_into_mihomo_yaml() {
        let result = normalize_subscription_text("ss://YWVzLTI1Ni1nY206cGFzc0BleGFtcGxlLmNvbTo0NDM=#demo")
            .expect("ss uri should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        assert_eq!(proxies.len(), 1);
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(first.get("type").and_then(serde_yaml_ng::Value::as_str), Some("ss"));
        assert_eq!(first.get("name").and_then(serde_yaml_ng::Value::as_str), Some("demo"));
        assert!(yaml.contains_key("proxy-groups"));
        assert!(yaml.contains_key("rules"));
    }

    #[test]
    fn normalizes_base64_subscription_lines() {
        let raw = "ss://YWVzLTI1Ni1nY206cGFzc0BleGFtcGxlLmNvbTo0NDM=#one\nsocks5://user:pass@127.0.0.1:1080#two";
        let encoded = STANDARD.encode(raw.as_bytes());
        let result = normalize_subscription_text(&encoded).expect("base64 subscription should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        assert_eq!(proxies.len(), 2);
    }

    #[test]
    fn passes_through_existing_mihomo_yaml() {
        let source = "proxies:\n  - name: direct-1\n    type: ss\n    server: 1.1.1.1\n    port: 443\n    cipher: aes-256-gcm\n    password: pass\n";
        let result = normalize_subscription_text(source).expect("yaml should pass through");
        assert_eq!(result.yaml, source.trim());
    }

    #[test]
    fn parses_vmess_json_with_ws_transport() {
        let payload = STANDARD.encode(
            r#"{"v":"2","ps":"demo","add":"example.com","port":"443","id":"12345678-1234-1234-1234-123456789012","aid":"0","scy":"auto","net":"ws","host":"cdn.example.com","path":"/ws","tls":"tls","sni":"sni.example.com"}"#,
        );
        let result = normalize_subscription_text(&format!("vmess://{payload}")).expect("vmess uri should parse");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(first.get("type").and_then(serde_yaml_ng::Value::as_str), Some("vmess"));
        assert_eq!(first.get("network").and_then(serde_yaml_ng::Value::as_str), Some("ws"));
        assert!(first.contains_key("ws-opts"));
    }

    #[test]
    fn looks_like_inline_source_for_share_links_and_yaml() {
        assert!(looks_like_inline_source(
            "ss://YWVzLTI1Ni1nY206cGFzc0BleGFtcGxlLmNvbTo0NDM=#demo"
        ));
        assert!(looks_like_inline_source(
            "proxies:\n  - name: demo\n    type: ss\n    server: 1.1.1.1\n    port: 443\n    cipher: aes-256-gcm\n    password: pass\n"
        ));
        assert!(!looks_like_inline_source("https://example.com/sub.yaml"));
    }

    #[test]
    fn translates_sing_box_json_outbounds() {
        let source = r#"{
          "outbounds": [
            {
              "type": "vmess",
              "tag": "sg-vmess",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto",
              "tls": {
                "enabled": true,
                "server_name": "cdn.example.com",
                "insecure": true
              },
              "transport": {
                "type": "ws",
                "path": "/ws",
                "headers": {
                  "Host": "cdn.example.com"
                }
              }
            }
          ]
        }"#;

        let result = normalize_subscription_text(source).expect("sing-box json should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(first.get("type").and_then(serde_yaml_ng::Value::as_str), Some("vmess"));
        assert_eq!(first.get("network").and_then(serde_yaml_ng::Value::as_str), Some("ws"));
        assert_eq!(first.get("tls").and_then(serde_yaml_ng::Value::as_bool), Some(true));
    }

    #[test]
    fn translates_xray_json_outbounds() {
        let source = r#"{
          "outbounds": [
            {
              "protocol": "trojan",
              "tag": "xray-trojan",
              "settings": {
                "servers": [
                  {
                    "address": "trojan.example.com",
                    "port": 443,
                    "password": "pass"
                  }
                ]
              },
              "streamSettings": {
                "network": "grpc",
                "security": "tls",
                "tlsSettings": {
                  "serverName": "cdn.example.com"
                },
                "grpcSettings": {
                  "serviceName": "grpc-service"
                }
              }
            }
          ]
        }"#;

        let result = normalize_subscription_text(source).expect("xray json should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(first.get("type").and_then(serde_yaml_ng::Value::as_str), Some("trojan"));
        assert_eq!(
            first.get("network").and_then(serde_yaml_ng::Value::as_str),
            Some("grpc")
        );
        assert!(first.contains_key("grpc-opts"));
    }

    #[test]
    fn translates_surge_proxy_section() {
        let source = r#"
[Proxy]
VMess = vmess, vmess.example.com, 443, auto, "12345678-1234-1234-1234-123456789012", obfs=wss, obfs-host=cdn.example.com, obfs-uri=/ws, tls-verification=false
SS = ss, ss.example.com, 8388, encrypt-method=aes-256-gcm, password=pass
"#;

        let result = normalize_subscription_text(source).expect("surge config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        assert_eq!(proxies.len(), 2);
    }

    #[test]
    fn translates_quantumult_server_local() {
        let source = r#"
[server_local]
vmess=vmess.example.com:443, method=auto, password=12345678-1234-1234-1234-123456789012, obfs=wss, obfs-host=cdn.example.com, obfs-uri=/ws, tag=QX VMess
"#;

        let result = normalize_subscription_text(source).expect("quantumult config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(
            first.get("name").and_then(serde_yaml_ng::Value::as_str),
            Some("QX VMess")
        );
        assert_eq!(first.get("type").and_then(serde_yaml_ng::Value::as_str), Some("vmess"));
    }

    #[test]
    fn parses_hysteria_share_link_alias_fields() {
        let source =
            "hy://hy.example.com:443?auth-str=token&ports=2000-3000&up=20&down=100&sni=cdn.example.com#Hy Demo";

        let result = normalize_subscription_text(source).expect("hysteria uri should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(
            first.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("hysteria")
        );
        assert_eq!(
            first.get("auth-str").and_then(serde_yaml_ng::Value::as_str),
            Some("token")
        );
        assert_eq!(
            first.get("ports").and_then(serde_yaml_ng::Value::as_str),
            Some("2000-3000")
        );
        assert_eq!(first.get("up").and_then(serde_yaml_ng::Value::as_str), Some("20"));
        assert_eq!(first.get("down").and_then(serde_yaml_ng::Value::as_str), Some("100"));
        assert_eq!(
            first.get("sni").and_then(serde_yaml_ng::Value::as_str),
            Some("cdn.example.com")
        );
    }

    #[test]
    fn parses_anytls_share_link() {
        let source =
            "anytls://secret@anytls.example.com:8443?sni=cdn.example.com&alpn=h2,http/1.1&insecure=1#AnyTLS Demo";

        let result = normalize_subscription_text(source).expect("anytls uri should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(first.get("type").and_then(serde_yaml_ng::Value::as_str), Some("anytls"));
        assert_eq!(
            first.get("sni").and_then(serde_yaml_ng::Value::as_str),
            Some("cdn.example.com")
        );
        assert_eq!(
            first.get("skip-cert-verify").and_then(serde_yaml_ng::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn parses_ssh_and_snell_share_links() {
        let ssh = normalize_subscription_text("ssh://user:pass@ssh.example.com:22?host-key=abc#SSH Demo")
            .expect("ssh uri should normalize");
        let ssh_yaml = parse_generated_yaml(&ssh.yaml);
        let ssh_proxy = ssh_yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist")[0]
            .as_mapping()
            .expect("proxy item should be mapping");
        assert_eq!(
            ssh_proxy.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("ssh")
        );
        assert_eq!(
            ssh_proxy.get("username").and_then(serde_yaml_ng::Value::as_str),
            Some("user")
        );

        let snell = normalize_subscription_text("snell://secret@snell.example.com:443?version=3&udp=1#Snell Demo")
            .expect("snell uri should normalize");
        let snell_yaml = parse_generated_yaml(&snell.yaml);
        let snell_proxy = snell_yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist")[0]
            .as_mapping()
            .expect("proxy item should be mapping");
        assert_eq!(
            snell_proxy.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("snell")
        );
        assert_eq!(
            snell_proxy.get("version").and_then(serde_yaml_ng::Value::as_u64),
            Some(3)
        );
    }

    #[test]
    fn parses_tail_protocol_share_links() {
        let source = r#"
mieru://user:pass@mieru.example.com:8443?transport=tcp&multiplexing=multiplexing_high&udp=1#Mieru%20Demo
masque://priv@masque.example.com:443?public-key=pub&ip=10.0.0.2&dns=1.1.1.1,8.8.8.8&remote-dns-resolve=1#Masque%20Demo
sudoku://secret@sudoku.example.com:443?aead-method=aes-128-gcm&padding-min=10&http-mask=1&http-mask-mode=stream&custom-tables=alpha,beta#Sudoku%20Demo
"#;

        let result = normalize_subscription_text(source).expect("tail protocol uris should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");

        let mieru = proxies[0].as_mapping().expect("mieru proxy should be mapping");
        assert_eq!(mieru.get("type").and_then(serde_yaml_ng::Value::as_str), Some("mieru"));
        assert_eq!(
            mieru.get("transport").and_then(serde_yaml_ng::Value::as_str),
            Some("TCP")
        );
        assert_eq!(
            mieru.get("multiplexing").and_then(serde_yaml_ng::Value::as_str),
            Some("MULTIPLEXING_HIGH")
        );
        assert_eq!(mieru.get("udp").and_then(serde_yaml_ng::Value::as_bool), Some(true));

        let masque = proxies[1].as_mapping().expect("masque proxy should be mapping");
        assert_eq!(
            masque.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("masque")
        );
        assert_eq!(
            masque.get("public-key").and_then(serde_yaml_ng::Value::as_str),
            Some("pub")
        );
        assert_eq!(
            masque.get("remote-dns-resolve").and_then(serde_yaml_ng::Value::as_bool),
            Some(true)
        );

        let sudoku = proxies[2].as_mapping().expect("sudoku proxy should be mapping");
        assert_eq!(
            sudoku.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("sudoku")
        );
        assert_eq!(
            sudoku.get("aead-method").and_then(serde_yaml_ng::Value::as_str),
            Some("aes-128-gcm")
        );
        assert_eq!(
            sudoku.get("http-mask-mode").and_then(serde_yaml_ng::Value::as_str),
            Some("stream")
        );
    }

    #[test]
    fn translates_surge_shadowsocksr_proxy_section() {
        let source = r#"
[Proxy]
SSR = shadowsocksr, ssr.example.com, 443, encrypt-method=aes-256-gcm, password=pass, obfs=tls1.2_ticket_auth, obfs-host=cdn.example.com, protocol=auth_aes128_md5, protocol-param=user:pass
"#;

        let result = normalize_subscription_text(source).expect("ssr config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(first.get("type").and_then(serde_yaml_ng::Value::as_str), Some("ssr"));
        assert_eq!(
            first.get("cipher").and_then(serde_yaml_ng::Value::as_str),
            Some("aes-256-gcm")
        );
        assert_eq!(
            first.get("obfs-param").and_then(serde_yaml_ng::Value::as_str),
            Some("cdn.example.com")
        );
        assert_eq!(
            first.get("protocol-param").and_then(serde_yaml_ng::Value::as_str),
            Some("user:pass")
        );
    }

    #[test]
    fn translates_quantumult_hysteria_alias_and_sni_fields() {
        let source = r#"
[server_local]
hy=hy.example.com:8443, auth-str=token, up=20, down=100, peer=cdn.example.com, tls-verification=false, tag=QX Hy
"#;

        let result = normalize_subscription_text(source).expect("hysteria config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(first.get("name").and_then(serde_yaml_ng::Value::as_str), Some("QX Hy"));
        assert_eq!(
            first.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("hysteria")
        );
        assert_eq!(
            first.get("auth-str").and_then(serde_yaml_ng::Value::as_str),
            Some("token")
        );
        assert_eq!(
            first.get("sni").and_then(serde_yaml_ng::Value::as_str),
            Some("cdn.example.com")
        );
        assert_eq!(
            first.get("skip-cert-verify").and_then(serde_yaml_ng::Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn translates_surge_ssh_and_anytls_proxy_sections() {
        let source = r#"
[Proxy]
SSH = ssh, ssh.example.com, 22, username=user, password=pass, host-key=abc
ATLS = anytls, anytls.example.com, 443, password=secret, sni=cdn.example.com
"#;

        let result = normalize_subscription_text(source).expect("ssh/anytls config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        assert_eq!(proxies.len(), 2);
        let ssh = proxies[0].as_mapping().expect("ssh proxy should be mapping");
        let anytls = proxies[1].as_mapping().expect("anytls proxy should be mapping");
        assert_eq!(ssh.get("type").and_then(serde_yaml_ng::Value::as_str), Some("ssh"));
        assert_eq!(
            anytls.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("anytls")
        );
        assert_eq!(
            anytls.get("sni").and_then(serde_yaml_ng::Value::as_str),
            Some("cdn.example.com")
        );
    }

    #[test]
    fn translates_surge_snell_proxy_section() {
        let source = r#"
[Proxy]
SN = snell, snell.example.com, 443, psk=secret, version=3, udp=true
"#;

        let result = normalize_subscription_text(source).expect("snell config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        let first = proxies[0].as_mapping().expect("proxy item should be mapping");
        assert_eq!(first.get("type").and_then(serde_yaml_ng::Value::as_str), Some("snell"));
        assert_eq!(first.get("psk").and_then(serde_yaml_ng::Value::as_str), Some("secret"));
        assert_eq!(first.get("version").and_then(serde_yaml_ng::Value::as_u64), Some(3));
    }

    #[test]
    fn translates_surge_proxy_groups_and_rules() {
        let source = r#"
[Proxy]
SS = ss, ss.example.com, 8388, encrypt-method=aes-256-gcm, password=pass
TR = trojan, trojan.example.com, 443, password=pass
[Proxy Group]
Proxy = select, SS, TR
Auto = url-test, SS, TR, url=http://www.gstatic.com/generate_204, interval=300
[Rule]
DOMAIN-SUFFIX,example.com,Proxy
FINAL,Auto
"#;

        let result = normalize_subscription_text(source).expect("surge config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let groups = yaml
            .get("proxy-groups")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxy-groups should exist");
        assert_eq!(groups.len(), 2);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(rules[0].as_str(), Some("DOMAIN-SUFFIX,example.com,Proxy"));
        assert_eq!(rules[1].as_str(), Some("MATCH,Auto"));
    }

    #[test]
    fn translates_surge_policy_path_group_into_proxy_provider() {
        let source = r#"
[Proxy Group]
Remote = select, policy-path=https://example.com/providers/remote.yaml, interval=3600
"#;

        let result = normalize_subscription_text(source).expect("surge policy-path should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let providers = yaml
            .get("proxy-providers")
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("proxy-providers should exist");
        assert_eq!(providers.len(), 1);
        let groups = yaml
            .get("proxy-groups")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxy-groups should exist");
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn translates_quantumult_custom_groups_and_rules() {
        let source = r#"
vmess=vmess.example.com:443, method=auto, password=12345678-1234-1234-1234-123456789012, tag=VM
custom_proxy_group=Proxy`select`[]VM`[]DIRECT
custom_proxy_group=Auto`url-test`[]VM`http://www.gstatic.com/generate_204`300
[filter_local]
host-suffix,example.com,Proxy
final,Auto
"#;

        let result = normalize_subscription_text(source).expect("qx config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let groups = yaml
            .get("proxy-groups")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxy-groups should exist");
        assert_eq!(groups.len(), 2);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(rules[0].as_str(), Some("DOMAIN-SUFFIX,example.com,Proxy"));
        assert_eq!(rules[1].as_str(), Some("MATCH,Auto"));
    }

    #[test]
    fn translates_sing_box_groups_and_route_rules() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a", "direct"]
            },
            {
              "type": "urltest",
              "tag": "Auto",
              "outbounds": ["vmess-a"],
              "url": "http://www.gstatic.com/generate_204",
              "interval": "5m"
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rules": [
              {
                "domain_suffix": ["example.com"],
                "outbound": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("sing-box config should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let groups = yaml
            .get("proxy-groups")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxy-groups should exist");
        assert_eq!(groups.len(), 2);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(rules[0].as_str(), Some("DOMAIN-SUFFIX,example.com,Proxy"));
    }

    #[test]
    fn translates_surge_ruleset_into_rule_provider() {
        let source = r#"
[Proxy]
SS = ss, ss.example.com, 8388, encrypt-method=aes-256-gcm, password=pass
[Rule]
RULE-SET,https://example.com/rules/list.txt,SS,no-resolve
"#;

        let result = normalize_subscription_text(source).expect("surge ruleset should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rule_providers = yaml
            .get("rule-providers")
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("rule-providers should exist");
        assert_eq!(rule_providers.len(), 1);
        let provider = rule_providers
            .get("list")
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("provider should exist");
        assert_eq!(
            provider.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("http")
        );
        assert_eq!(
            provider.get("format").and_then(serde_yaml_ng::Value::as_str),
            Some("text")
        );
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(rules[0].as_str(), Some("RULE-SET,list,SS,no-resolve"));
    }

    #[test]
    fn translates_quantumult_filter_remote_into_rule_provider() {
        let source = r#"
vmess=vmess.example.com:443, method=auto, password=12345678-1234-1234-1234-123456789012, tag=VM
[filter_remote]
https://example.com/rules/qx.list, tag=QXRemote, force-policy=VM, enabled=true, update-interval=86400
"#;

        let result = normalize_subscription_text(source).expect("qx remote rules should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rule_providers = yaml
            .get("rule-providers")
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("rule-providers should exist");
        assert!(rule_providers.contains_key("QXRemote"));
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(rules[0].as_str(), Some("RULE-SET,QXRemote,VM"));
    }

    #[test]
    fn translates_quantumult_server_remote_into_proxy_provider() {
        let source = r#"
[server_remote]
https://example.com/subscription.yaml, tag=RemoteSub, update-interval=86400
"#;

        let result = normalize_subscription_text(source).expect("server_remote should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let providers = yaml
            .get("proxy-providers")
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("proxy-providers should exist");
        assert!(providers.contains_key("RemoteSub"));
        let groups = yaml
            .get("proxy-groups")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxy-groups should exist");
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn translates_surge_policy_path_file_into_proxy_provider() {
        let source = r#"
[Proxy Group]
Remote = select, policy-path=./providers/remote.list
"#;

        let result = normalize_subscription_text(source).expect("surge file policy-path should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let providers = yaml
            .get("proxy-providers")
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("proxy-providers should exist");
        let provider = providers
            .values()
            .next()
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("provider should exist");
        assert_eq!(
            provider.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("file")
        );
    }

    #[test]
    fn translates_sing_box_multi_match_rule_into_and_expression() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a"]
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rules": [
              {
                "domain_suffix": ["example.com"],
                "network": "tcp",
                "outbound": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("sing-box and-rule should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(
            rules[0].as_str(),
            Some("AND,((DOMAIN-SUFFIX,example.com),(NETWORK,tcp)),Proxy")
        );
    }

    #[test]
    fn translates_sing_box_multi_value_matchers_into_or_and_expression() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a"]
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rules": [
              {
                "domain_suffix": ["a.com", "b.com"],
                "network": "tcp",
                "outbound": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("sing-box multi-value rule should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(
            rules[0].as_str(),
            Some("AND,((OR,((DOMAIN-SUFFIX,a.com),(DOMAIN-SUFFIX,b.com))),(NETWORK,tcp)),Proxy")
        );
    }

    #[test]
    fn translates_sing_box_invert_rule_into_not_expression() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a"]
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rules": [
              {
                "domain_suffix": ["example.com"],
                "invert": true,
                "outbound": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("sing-box invert rule should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(rules[0].as_str(), Some("NOT,((DOMAIN-SUFFIX,example.com)),Proxy"));
    }

    #[test]
    fn translates_sing_box_rule_set_into_rule_provider() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a"]
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rule_set": [
              {
                "tag": "ads",
                "type": "remote",
                "url": "https://example.com/ads.srs",
                "update_interval": "1h",
                "download_detour": "Proxy"
              }
            ],
            "rules": [
              {
                "rule_set": ["ads"],
                "outbound": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("sing-box rule-set should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let providers = yaml
            .get("rule-providers")
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("rule-providers should exist");
        assert!(providers.contains_key("ads"));
        let provider = providers
            .get("ads")
            .and_then(serde_yaml_ng::Value::as_mapping)
            .expect("ads provider should exist");
        assert_eq!(
            provider.get("type").and_then(serde_yaml_ng::Value::as_str),
            Some("http")
        );
        assert_eq!(
            provider.get("format").and_then(serde_yaml_ng::Value::as_str),
            Some("mrs")
        );
        assert_eq!(
            provider.get("proxy").and_then(serde_yaml_ng::Value::as_str),
            Some("Proxy")
        );
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(rules[0].as_str(), Some("RULE-SET,ads,Proxy"));
    }

    #[test]
    fn translates_xray_multi_match_rule_into_and_expression() {
        let source = r#"{
          "outbounds": [
            {
              "protocol": "vmess",
              "tag": "Proxy",
              "settings": {
                "vnext": [
                  {
                    "address": "vmess.example.com",
                    "port": 443,
                    "users": [
                      {
                        "id": "12345678-1234-1234-1234-123456789012",
                        "security": "auto"
                      }
                    ]
                  }
                ]
              }
            }
          ],
          "routing": {
            "rules": [
              {
                "domain": ["domain:example.com"],
                "network": "tcp",
                "outboundTag": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("xray and-rule should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(
            rules[0].as_str(),
            Some("AND,((DOMAIN-SUFFIX,example.com),(NETWORK,tcp)),Proxy")
        );
    }

    #[test]
    fn translates_xray_inbound_and_port_rules() {
        let source = r#"{
          "outbounds": [
            {
              "protocol": "vmess",
              "tag": "Proxy",
              "settings": {
                "vnext": [
                  {
                    "address": "vmess.example.com",
                    "port": 443,
                    "users": [
                      {
                        "id": "12345678-1234-1234-1234-123456789012",
                        "security": "auto"
                      }
                    ]
                  }
                ]
              }
            }
          ],
          "routing": {
            "rules": [
              {
                "inboundTag": ["socks-in"],
                "localPort": "7890",
                "outboundTag": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("xray inbound rules should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(rules[0].as_str(), Some("AND,((IN-PORT,7890),(IN-NAME,socks-in)),Proxy"));
    }

    #[test]
    fn translates_tail_protocol_proxy_section_entries() {
        let source = r#"
[Proxy]
Mieru = mieru, mieru.example.com, 443, username=user, password=pass, transport=TCP, handshake-mode=MULTIPLEXING_HIGH
Masque = masque, masque.example.com, 443, private-key=priv, public-key=pub, ip=10.0.0.2, udp=true
Sudoku = sudoku, sudoku.example.com, 443, key=secret, aead-method=chacha20-poly1305, padding-min=10, padding-max=20
"#;

        let result = normalize_subscription_text(source).expect("tail protocol entries should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let proxies = yaml
            .get("proxies")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxies should exist");
        assert_eq!(proxies.len(), 3);
        assert_eq!(
            proxies[0]
                .as_mapping()
                .and_then(|mapping| mapping.get("type"))
                .and_then(serde_yaml_ng::Value::as_str),
            Some("mieru")
        );
        assert_eq!(
            proxies[1]
                .as_mapping()
                .and_then(|mapping| mapping.get("type"))
                .and_then(serde_yaml_ng::Value::as_str),
            Some("masque")
        );
        assert_eq!(
            proxies[2]
                .as_mapping()
                .and_then(|mapping| mapping.get("type"))
                .and_then(serde_yaml_ng::Value::as_str),
            Some("sudoku")
        );
    }

    #[test]
    fn translates_sing_box_package_and_mode_rules() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a"]
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rules": [
              {
                "package_name": ["com.example.app"],
                "clash_mode": ["global"],
                "outbound": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("sing-box package rule should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(
            rules[0].as_str(),
            Some("AND,((PROCESS-NAME,com.example.app),(IN-NAME,global)),Proxy")
        );
    }

    #[test]
    fn translates_sing_box_private_ip_rule() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a"]
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rules": [
              {
                "ip_is_private": true,
                "outbound": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("sing-box private ip rule should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");
        assert_eq!(
            rules[0].as_str(),
            Some("OR,((IP-CIDR,10.0.0.0/8),(IP-CIDR,172.16.0.0/12),(IP-CIDR,192.168.0.0/16),(IP-CIDR,fc00::/7)),Proxy")
        );
    }

    #[test]
    fn includes_translation_notes_for_unsupported_high_risk_fields() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a"]
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rules": [
              {
                "domain_suffix": ["example.com"],
                "network_type": ["wifi"],
                "outbound": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("unsupported fields should still normalize");
        assert!(result.yaml.contains("# Translation notes:"));
        assert!(result.yaml.contains("network_type"));
    }

    #[test]
    fn includes_translation_notes_for_surge_extended_matching() {
        let source = r#"
[Proxy]
SS = ss, ss.example.com, 8388, encrypt-method=aes-256-gcm, password=pass
[Rule]
RULE-SET,https://example.com/rules/list.txt,SS,extended-matching
"#;

        let result = normalize_subscription_text(source).expect("surge extended matching should still normalize");
        assert!(result.yaml.contains("extended-matching"));
    }

    #[test]
    fn includes_translation_notes_for_xray_attrs_webhook() {
        let source = r#"{
          "outbounds": [
            {
              "protocol": "vmess",
              "tag": "Proxy",
              "settings": {
                "vnext": [
                  {
                    "address": "vmess.example.com",
                    "port": 443,
                    "users": [
                      {
                        "id": "12345678-1234-1234-1234-123456789012",
                        "security": "auto"
                      }
                    ]
                  }
                ]
              }
            }
          ],
          "routing": {
            "rules": [
              {
                "domain": ["domain:example.com"],
                "attrs": ":method=GET",
                "webhook": "https://example.com/hook",
                "outboundTag": "Proxy"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("xray attrs/webhook should still normalize");
        assert!(result.yaml.contains("attrs"));
        assert!(result.yaml.contains("webhook"));
    }

    #[test]
    fn keeps_relay_groups_free_of_auto_direct_members() {
        let source = r#"
[Proxy]
NodeA = ss, a.example.com, 8388, encrypt-method=aes-256-gcm, password=pass
NodeB = ss, b.example.com, 8388, encrypt-method=aes-256-gcm, password=pass
[Proxy Group]
Chain = relay, NodeA, NodeB
"#;

        let result = normalize_subscription_text(source).expect("relay group should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let groups = yaml
            .get("proxy-groups")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("proxy groups should exist");
        let proxies = groups[0]
            .as_mapping()
            .and_then(|mapping| mapping.get("proxies"))
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("relay proxies should exist");

        assert_eq!(proxies.len(), 2);
        assert_eq!(proxies[0].as_str(), Some("NodeA"));
        assert_eq!(proxies[1].as_str(), Some("NodeB"));
    }

    #[test]
    fn includes_translation_notes_for_surge_external_policy_fields() {
        let source = r#"
[Proxy Group]
Remote = select, policy-path=https://example.com/proxies.yaml, external-policy=HK, external-policy-modifier=regex
"#;

        let result = normalize_subscription_text(source).expect("surge external policy options should normalize");
        assert!(result.yaml.contains("external-policy"));
        assert!(result.yaml.contains("external-policy-modifier"));
    }

    #[test]
    fn includes_translation_notes_for_sing_box_unsupported_actions() {
        let source = r#"{
          "outbounds": [
            {
              "type": "selector",
              "tag": "Proxy",
              "outbounds": ["vmess-a"]
            },
            {
              "type": "vmess",
              "tag": "vmess-a",
              "server": "vmess.example.com",
              "server_port": 443,
              "uuid": "12345678-1234-1234-1234-123456789012",
              "security": "auto"
            }
          ],
          "route": {
            "rules": [
              {
                "protocol": ["dns"],
                "action": "sniff"
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("unsupported sing-box action should still normalize");
        assert!(result.yaml.contains("action `sniff`"));
        assert!(result.yaml.contains("protocol"));
    }

    #[test]
    fn translates_xray_exact_ip_rules_and_warns_on_ext_matchers() {
        let source = r#"{
          "outbounds": [
            {
              "protocol": "vmess",
              "tag": "Proxy",
              "settings": {
                "vnext": [
                  {
                    "address": "vmess.example.com",
                    "port": 443,
                    "users": [
                      {
                        "id": "12345678-1234-1234-1234-123456789012",
                        "security": "auto"
                      }
                    ]
                  }
                ]
              }
            }
          ],
          "routing": {
            "rules": [
              {
                "ip": ["1.1.1.1", "2001:db8::1"],
                "outboundTag": "Proxy"
              },
              {
                "domain": ["ext:geo.dat:google"]
              }
            ]
          }
        }"#;

        let result = normalize_subscription_text(source).expect("xray exact ip rules should normalize");
        let yaml = parse_generated_yaml(&result.yaml);
        let rules = yaml
            .get("rules")
            .and_then(serde_yaml_ng::Value::as_sequence)
            .expect("rules should exist");

        assert_eq!(
            rules[0].as_str(),
            Some("OR,((IP-CIDR,1.1.1.1/32),(IP-CIDR6,2001:db8::1/128)),Proxy")
        );
        assert!(result.yaml.contains("ext:geo.dat:google"));
        assert!(result.yaml.contains("without `outboundTag` or `balancerTag`"));
    }
}
