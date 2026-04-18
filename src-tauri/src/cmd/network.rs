use super::CmdResult;
use crate::cmd::StringifyErr as _;
use clash_verge_logging::{Type, logging};
use gethostname::gethostname;
use network_interface::NetworkInterface;
use serde_yaml_ng::Mapping;
use std::net::TcpListener;
use sysproxy::{Autoproxy, Sysproxy};
use tauri_plugin_clash_verge_sysinfo;

fn is_missing_default_network_interface(message: &str) -> bool {
    message.contains("failed to get default network interface")
        || message.contains("failed to parse string `failed to get default network interface`")
}

fn sysproxy_mapping(enable: bool, server: &str, bypass: &str) -> Mapping {
    let mut map = Mapping::new();
    map.insert("enable".into(), enable.into());
    map.insert("server".into(), server.into());
    map.insert("bypass".into(), bypass.into());
    map
}

fn autoproxy_mapping(enable: bool, url: &str) -> Mapping {
    let mut map = Mapping::new();
    map.insert("enable".into(), enable.into());
    map.insert("url".into(), url.into());
    map
}

/// get the system proxy
#[tauri::command]
pub async fn get_sys_proxy() -> CmdResult<Mapping> {
    logging!(debug, Type::Network, "异步获取系统代理配置");

    let sys_proxy = match Sysproxy::get_system_proxy() {
        Ok(sys_proxy) => sys_proxy,
        Err(err) => {
            let message = err.to_string();
            if is_missing_default_network_interface(&message) {
                logging!(
                    debug,
                    Type::Network,
                    "系统代理未绑定到默认网卡，返回空配置而不是错误: {}",
                    message
                );
                return Ok(sysproxy_mapping(false, "", ""));
            }
            return Err(message.into());
        }
    };
    let Sysproxy {
        ref host,
        ref bypass,
        ref port,
        ref enable,
    } = sys_proxy;

    logging!(
        debug,
        Type::Network,
        "返回系统代理配置: enable={}, {}:{}",
        sys_proxy.enable,
        sys_proxy.host,
        sys_proxy.port
    );
    Ok(sysproxy_mapping(
        *enable,
        &format!("{}:{}", host, port),
        bypass.as_str(),
    ))
}

/// 获取自动代理配置
#[tauri::command]
pub async fn get_auto_proxy() -> CmdResult<Mapping> {
    let auto_proxy = match Autoproxy::get_auto_proxy() {
        Ok(auto_proxy) => auto_proxy,
        Err(err) => {
            let message = err.to_string();
            if message.contains("Proxy settings not found in DynamicStore")
                || message.contains("failed to parse string `Proxy settings not found in DynamicStore`")
                || is_missing_default_network_interface(&message)
            {
                logging!(
                    debug,
                    Type::Network,
                    "自动代理未配置，返回空配置而不是错误: {}",
                    message
                );
                return Ok(autoproxy_mapping(false, ""));
            }
            return Err(message.into());
        }
    };
    let Autoproxy { ref enable, ref url } = auto_proxy;

    logging!(
        debug,
        Type::Network,
        "返回自动代理配置（缓存）: enable={}, url={}",
        auto_proxy.enable,
        auto_proxy.url
    );
    Ok(autoproxy_mapping(*enable, url))
}

/// 获取系统主机名
#[tauri::command]
pub fn get_system_hostname() -> String {
    // 获取系统主机名，处理可能的非UTF-8字符
    match gethostname().into_string() {
        Ok(name) => name,
        Err(os_string) => {
            // 对于包含非UTF-8的主机名，使用调试格式化
            let fallback = format!("{os_string:?}");
            // 去掉可能存在的引号
            fallback.trim_matches('"').to_string()
        }
    }
}

/// 获取网络接口列表
#[tauri::command]
pub fn get_network_interfaces() -> Vec<String> {
    tauri_plugin_clash_verge_sysinfo::list_network_interfaces()
}

/// 获取网络接口详细信息
#[tauri::command]
pub fn get_network_interfaces_info() -> CmdResult<Vec<NetworkInterface>> {
    use network_interface::{NetworkInterface, NetworkInterfaceConfig as _};

    let names = get_network_interfaces();
    let interfaces = NetworkInterface::show().stringify_err()?;

    let mut result = Vec::new();

    for interface in interfaces {
        if names.contains(&interface.name) {
            result.push(interface);
        }
    }

    Ok(result)
}

#[tauri::command]
pub fn is_port_in_use(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_err()
}
