use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SmartMonResults {
	pub serial_number: String,
	pub smart_status: SmartMonStatus,
	pub model_family: String,
	pub model_name: String,
	pub temperature: SmartMonTemp,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SmartMonStatus {
	pub passed: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SmartMonTemp {
	pub current: u32,
}
