use std::borrow::Cow;

type CowStr = Cow<'static, str>;

#[derive(Debug, Clone)]
pub struct HassNode {
	pub name: CowStr,
	pub sw_version: CowStr,
	pub support_url: CowStr,
	pub entities: Vec<HassEntity>,
}

#[derive(Debug, Clone)]
pub enum HassEntity {
	Sensor {
		availability_topic: Option<CowStr>,
		device: Option<HassDevice>,
	}
}

#[derive(Debug, Clone)]
pub struct HassDevice {
	pub configuration_url: Option<CowStr>,
	pub connections: Option<Vec<(CowStr, CowStr)>>,
	pub hw_version: Option<CowStr>,
	pub identifiers: Option<Vec<CowStr>>,
	pub manufacturer: Option<String>,
}
