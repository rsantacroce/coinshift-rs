use eframe::egui::{self, Button, Color32, ComboBox, RichText};
use coinshift::types::{Address, ParentChainType};
use poll_promise::Promise;
use serde_json::json;

use crate::app::App;

#[derive(Clone, Debug)]
enum ConnectionStatus {
    Unknown,
    Connected { block_height: u64 },
    Disconnected { error: String },
    Checking,
}

pub struct CreateSwap {
    parent_chain: ParentChainType,
    l1_recipient_address: String,
    l1_amount: String,
    l2_recipient: Option<String>,
    l2_amount: String,
    required_confirmations: String,
    is_open_swap: bool,
    error_message: Option<String>,

    rpc_status: ConnectionStatus,
    rpc_status_promise: Option<Promise<anyhow::Result<u64>>>,
}

impl std::fmt::Debug for CreateSwap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateSwap")
            .field("parent_chain", &self.parent_chain)
            .field("l1_recipient_address", &self.l1_recipient_address)
            .field("l1_amount", &self.l1_amount)
            .field("l2_recipient", &self.l2_recipient)
            .field("l2_amount", &self.l2_amount)
            .field("required_confirmations", &self.required_confirmations)
            .field("is_open_swap", &self.is_open_swap)
            .field("error_message", &self.error_message)
            .field("rpc_status", &self.rpc_status)
            .field("rpc_status_promise", &"<Promise>")
            .finish()
    }
}

impl Default for CreateSwap {
    fn default() -> Self {
        Self {
            parent_chain: ParentChainType::BTC,
            l1_recipient_address: String::new(),
            l1_amount: String::new(),
            l2_recipient: None,
            l2_amount: String::new(),
            required_confirmations: String::new(),
            is_open_swap: false,
            error_message: None,

            rpc_status: ConnectionStatus::Unknown,
            rpc_status_promise: None,
        }
    }
}

impl CreateSwap {
    fn load_rpc_config(&self, parent_chain: ParentChainType) -> Option<coinshift::bitcoin_rpc::RpcConfig> {
        use std::collections::HashMap;
        use std::path::PathBuf;
        use serde::{Deserialize, Serialize};

        #[derive(Clone, Serialize, Deserialize)]
        struct LocalRpcConfig {
            url: String,
            user: String,
            password: String,
        }

        let config_path = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("coinshift")
            .join("l1_rpc_configs.json");

        if let Ok(file_content) = std::fs::read_to_string(&config_path) {
            if let Ok(configs) =
                serde_json::from_str::<HashMap<ParentChainType, LocalRpcConfig>>(&file_content)
            {
                if let Some(local_config) = configs.get(&parent_chain) {
                    return Some(coinshift::bitcoin_rpc::RpcConfig {
                        url: local_config.url.clone(),
                        user: local_config.user.clone(),
                        password: local_config.password.clone(),
                    });
                }
            }
        }
        None
    }

    fn fetch_block_height(url: &str, user: &str, password: &str) -> anyhow::Result<u64> {
        use std::time::Duration;

        tracing::debug!(
            url = %url,
            has_user = !user.is_empty(),
            "Starting RPC request to getblockchaininfo (Create Swap)"
        );

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .map_err(|e| {
                tracing::error!(error = %e, "Failed to create HTTP client (Create Swap)");
                e
            })?;

        let request = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getblockchaininfo",
            "params": []
        });

        if let Ok(request_str) = serde_json::to_string(&request) {
            tracing::debug!(request = %request_str, "Sending RPC request (Create Swap)");
        }

        let mut request_builder = client.post(url).json(&request);
        if !user.is_empty() {
            request_builder = request_builder.basic_auth(user, Some(password));
            tracing::debug!("Added HTTP basic authentication (Create Swap)");
        }

        let response = request_builder.send().map_err(|e| {
            tracing::error!(
                url = %url,
                error = %e,
                error_debug = ?e,
                "Failed to send RPC request (Create Swap)"
            );
            e
        })?;

        let status = response.status();
        tracing::debug!(status = %status, "Received HTTP response (Create Swap)");

        let json: serde_json::Value = response.json().map_err(|e| {
            tracing::error!(
                error = %e,
                error_debug = ?e,
                "Failed to parse JSON response (Create Swap)"
            );
            e
        })?;

        if let Ok(response_str) = serde_json::to_string_pretty(&json) {
            tracing::debug!(response = %response_str, "RPC response received (Create Swap)");
        } else {
            tracing::debug!("RPC response received (failed to serialize for logging) (Create Swap)");
        }

        // In JSON-RPC 2.0, error is null when there's no error, and an object when there is an error
        if let Some(error) = json.get("error") {
            if !error.is_null() {
                tracing::error!(error = %error, "RPC returned an error (Create Swap)");
                anyhow::bail!("RPC error: {}", error);
            }
        }

        let result = json
            .get("result")
            .ok_or_else(|| {
                tracing::error!("No 'result' field in RPC response (Create Swap)");
                anyhow::anyhow!("No result in response")
            })?;

        let blocks = result
            .get("blocks")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                tracing::error!(result = ?result, "No 'blocks' field in RPC result (Create Swap)");
                anyhow::anyhow!("No blocks field in response")
            })?;

        tracing::info!(blocks = blocks, "Successfully fetched block height from RPC (Create Swap)");
        Ok(blocks)
    }

    fn check_rpc_connection(&mut self) {
        let parent_chain = self.parent_chain;
        tracing::info!(
            parent_chain = ?parent_chain,
            "Checking parent chain RPC connection (Create Swap)"
        );

        let Some(cfg) = self.load_rpc_config(self.parent_chain) else {
            tracing::warn!(
                parent_chain = ?parent_chain,
                "No RPC config found for parent chain (Create Swap)"
            );
            self.rpc_status = ConnectionStatus::Disconnected {
                error: "No RPC config found for this parent chain. Set it in 'L1 Node RPC Configuration' first.".to_string(),
            };
            self.rpc_status_promise = None;
            return;
        };

        if cfg.url.trim().is_empty() {
            tracing::warn!(
                parent_chain = ?parent_chain,
                "RPC URL is empty (Create Swap)"
            );
            self.rpc_status = ConnectionStatus::Disconnected {
                error: "RPC URL is empty. Set it in 'L1 Node RPC Configuration' first.".to_string(),
            };
            self.rpc_status_promise = None;
            return;
        }

        tracing::info!(
            parent_chain = ?parent_chain,
            url = %cfg.url,
            user = %cfg.user,
            "Starting RPC connection check (Create Swap)"
        );

        let url = cfg.url.clone();
        let user = cfg.user.clone();
        let password = cfg.password.clone();

        self.rpc_status = ConnectionStatus::Checking;
        self.rpc_status_promise = Some(Promise::spawn_thread("swap_parent_chain_rpc_check", move || {
            Self::fetch_block_height(&url, &user, &password)
        }));
    }

    fn update_rpc_status(&mut self) {
        let Some(p) = self.rpc_status_promise.take() else { return };
        if let Some(res) = p.ready() {
            let parent_chain = self.parent_chain;
            match res {
                Ok(h) => {
                    tracing::info!(
                        parent_chain = ?parent_chain,
                        block_height = h,
                        "Parent chain RPC connection check succeeded (Create Swap)"
                    );
                    self.rpc_status = ConnectionStatus::Connected { block_height: *h };
                }
                Err(e) => {
                    tracing::error!(
                        parent_chain = ?parent_chain,
                        error = %e,
                        error_debug = ?e,
                        "Parent chain RPC connection check failed (Create Swap)"
                    );
                    self.rpc_status = ConnectionStatus::Disconnected { error: format!("{e:#}") }
                }
            }
        } else {
            self.rpc_status_promise = Some(p);
        }
    }

    pub fn show(&mut self, app: Option<&App>, ui: &mut egui::Ui) {
        ui.heading("Create Swap (L2 → L1)");
        ui.separator();

        // Parent chain selection
        ui.horizontal(|ui| {
            ui.label("Parent Chain:");
            let prev_chain = self.parent_chain;
            ComboBox::from_id_salt("parent_chain")
                .selected_text(format!("{:?}", self.parent_chain))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.parent_chain, ParentChainType::BTC, "BTC");
                    ui.selectable_value(&mut self.parent_chain, ParentChainType::Signet, "Signet");
                    ui.selectable_value(&mut self.parent_chain, ParentChainType::Regtest, "Regtest");
                    ui.selectable_value(&mut self.parent_chain, ParentChainType::BCH, "BCH");
                    ui.selectable_value(&mut self.parent_chain, ParentChainType::LTC, "LTC");
                    ui.selectable_value(&mut self.parent_chain, ParentChainType::ZEC, "ZEC (transparent)");
                });

            if prev_chain != self.parent_chain {
                // Reset status when switching chains.
                self.rpc_status = ConnectionStatus::Unknown;
                self.rpc_status_promise = None;
            }
        });

        // Parent-chain RPC connectivity (uses L1 Config stored per chain)
        self.update_rpc_status();
        ui.horizontal(|ui| {
            ui.label("Parent Chain RPC:");
            match self.rpc_status.clone() {
                ConnectionStatus::Unknown => {
                    ui.label(RichText::new("Unknown").color(Color32::GRAY));
                    if ui.button("Test Connection").clicked() {
                        tracing::info!(
                            parent_chain = ?self.parent_chain,
                            "User clicked Test Connection button (Create Swap)"
                        );
                        self.check_rpc_connection();
                    }
                }
                ConnectionStatus::Checking => {
                    ui.label(RichText::new("Checking...").color(Color32::YELLOW));
                }
                ConnectionStatus::Connected { block_height } => {
                    ui.label(RichText::new("Connected").color(Color32::GREEN).strong());
                    ui.label(format!("Height: {block_height}"));
                    if ui.button("Refresh").clicked() {
                        tracing::info!(
                            parent_chain = ?self.parent_chain,
                            "User clicked Refresh button (Create Swap)"
                        );
                        self.check_rpc_connection();
                    }
                }
                ConnectionStatus::Disconnected { error } => {
                    ui.label(RichText::new("Disconnected").color(Color32::RED).strong());
                    let error_clone = error.clone();
                    ui.label(RichText::new(error).small().color(Color32::RED));
                    if ui.button("Retry").clicked() {
                        tracing::info!(
                            parent_chain = ?self.parent_chain,
                            error = %error_clone,
                            "User clicked Retry button to check parent chain RPC connection (Create Swap)"
                        );
                        self.check_rpc_connection();
                    }
                }
            }
        });

        // L1 recipient address
        ui.horizontal(|ui| {
            ui.label("L1 Recipient Address:");
            ui.text_edit_singleline(&mut self.l1_recipient_address);
        });

        // L1 amount
        ui.horizontal(|ui| {
            let parent_chain_label = match self.parent_chain {
                ParentChainType::BTC => "BTC",
                ParentChainType::Signet => "Signet",
                ParentChainType::Regtest => "Regtest",
                ParentChainType::BCH => "BCH",
                ParentChainType::LTC => "LTC",
                ParentChainType::ZEC => "ZEC",
            };
            ui.label(format!("L1 Amount ({parent_chain_label}):"));
            ui.text_edit_singleline(&mut self.l1_amount);
        });

        // Open swap checkbox
        ui.checkbox(&mut self.is_open_swap, "Open Swap (anyone can fill)");

        // L2 recipient (only if not open swap)
        if !self.is_open_swap {
            ui.horizontal(|ui| {
                ui.label("L2 Recipient Address:");
                ui.text_edit_singleline(
                    self.l2_recipient.get_or_insert_with(String::new),
                );
                if ui.button("Use My Address").clicked() {
                    if let Some(app) = app {
                        match app.wallet.get_new_address() {
                            Ok(addr) => {
                                self.l2_recipient = Some(addr.to_string());
                            }
                            Err(err) => {
                                tracing::error!("Failed to get address: {err:#}");
                            }
                        }
                    }
                }
            });
        } else {
            self.l2_recipient = None;
        }

        // L2 amount
        ui.horizontal(|ui| {
            ui.label("L2 Amount (BTC):");
            ui.text_edit_singleline(&mut self.l2_amount);
        });

        // Required confirmations
        ui.horizontal(|ui| {
            ui.label("Required Confirmations:");
            ui.text_edit_singleline(&mut self.required_confirmations);
            ui.label(format!(
                "(Default: {})",
                self.parent_chain.default_confirmations()
            ));
        });

        ui.separator();

        // Display error message if any
        if let Some(error_msg) = &self.error_message {
            ui.add_space(5.0);
            ui.label(RichText::new(format!("Error: {}", error_msg)).small().color(Color32::RED));
            ui.separator();
        }

        // Parse inputs
        let l1_amount = bitcoin::Amount::from_str_in(
            &self.l1_amount,
            bitcoin::Denomination::Bitcoin,
        );
        let l2_amount = bitcoin::Amount::from_str_in(
            &self.l2_amount,
            bitcoin::Denomination::Bitcoin,
        );
        let required_confirmations = self
            .required_confirmations
            .parse::<u32>()
            .ok()
            .or_else(|| Some(self.parent_chain.default_confirmations()));

        let l2_recipient: Option<Address> = if self.is_open_swap {
            None
        } else {
            self.l2_recipient
                .as_ref()
                .and_then(|s| s.parse().ok())
        };

        let is_valid = app.is_some()
            && !self.l1_recipient_address.is_empty()
            && l1_amount.is_ok()
            && l2_amount.is_ok()
            && (!self.is_open_swap && l2_recipient.is_some() || self.is_open_swap);

        if ui
            .add_enabled(is_valid, Button::new("Create Swap"))
            .clicked()
        {
            // Clear any previous error
            self.error_message = None;
            
            let app = app.unwrap();
            let accumulator = match app.node.get_tip_accumulator() {
                Ok(acc) => acc,
                Err(err) => {
                    let error_msg = format!("Failed to get accumulator: {err:#}");
                    tracing::error!("{}", error_msg);
                    self.error_message = Some(error_msg);
                    return;
                }
            };

            // Extract amounts for logging (before they're moved)
            let l1_amount_val = l1_amount.expect("should not happen");
            let l2_amount_val = l2_amount.expect("should not happen");

            // Create a closure that checks if an outpoint is locked to a swap
            // We create a new read transaction each time to avoid lifetime issues
            let node = &app.node;
            let is_locked = |outpoint: &coinshift::types::OutPoint| -> bool {
                let rotxn = match node.env().read_txn() {
                    Ok(txn) => txn,
                    Err(_) => return false,
                };
                let state = node.state();
                state
                    .is_output_locked_to_swap(&rotxn, outpoint)
                    .map(|opt| opt.is_some())
                    .unwrap_or(false)
            };

            let (tx, swap_id) = match app.wallet.create_swap_create_tx(
                &accumulator,
                self.parent_chain,
                self.l1_recipient_address.clone(),
                l1_amount_val,
                l2_recipient,
                l2_amount_val,
                required_confirmations,
                bitcoin::Amount::ZERO,
                is_locked,
            ) {
                Ok(result) => {
                    let txid = result.0.txid();
                    tracing::info!(
                        swap_id = %result.1,
                        txid = %txid,
                        parent_chain = ?self.parent_chain,
                        l1_recipient = %self.l1_recipient_address,
                        l1_amount = %l1_amount_val,
                        l2_recipient = ?l2_recipient,
                        l2_amount = %l2_amount_val,
                        required_confirmations = ?required_confirmations,
                        is_open_swap = %self.is_open_swap,
                        "Successfully created swap transaction"
                    );
                    result
                }
                Err(err) => {
                    let error_msg = format!("Failed to create swap transaction: {err:#}");
                    tracing::error!(
                        parent_chain = ?self.parent_chain,
                        l1_recipient = %self.l1_recipient_address,
                        l1_amount = %l1_amount_val,
                        l2_recipient = ?l2_recipient,
                        l2_amount = %l2_amount_val,
                        required_confirmations = ?required_confirmations,
                        is_open_swap = %self.is_open_swap,
                        error = %err,
                        error_debug = ?err,
                        "Failed to create swap transaction"
                    );
                    self.error_message = Some(error_msg);
                    return;
                }
            };

            let txid = tx.txid();
            tracing::info!(
                swap_id = %swap_id,
                txid = %txid,
                "Attempting to sign and send swap transaction"
            );
            if let Err(err) = app.sign_and_send(tx) {
                let error_msg = format!("Failed to send transaction: {err:#}");
                tracing::error!(
                    swap_id = %swap_id,
                    txid = %txid,
                    error = %err,
                    error_debug = ?err,
                    "Failed to send transaction: node error"
                );
                self.error_message = Some(error_msg);
                return;
            }

            tracing::info!("Swap created: swap_id={}, txid={}", swap_id, txid);
            *self = Self::default();
            self.parent_chain = ParentChainType::BTC; // Keep parent chain selection
        }
    }
}

