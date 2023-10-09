mod config_types;
mod smartmon_types;
mod hass_sensors;

use std::{path::Path, fs::File, time::Duration, thread, process::{Command, Stdio}, io::Read, str::FromStr};

use anyhow::{Context, anyhow};
use config_types::MonDevice;
use rumqttc::{MqttOptions, QoS, Client};
use serde_json::json;
use clap::Parser;
use smartmon_types::SmartMonResults;

#[derive(Parser)]
/// Periodically reports the status of disks to Home Assistant
/// via MQTT
struct Args {
	#[arg(short)]
	/// The path to the config file
	config_file: String,
}

fn main() {
	let args = Args::parse();
	let config_file = File::open(args.config_file).expect("Couldn't open config file");
	let config: config_types::Config = serde_yaml::from_reader(config_file).expect("Couldn't read config file");

	eprintln!("Connecting to {}:{} as {}", &config.mqtt_host, config.mqtt_port.unwrap_or(1883), &config.node_id);
	let mut mqtt_options = MqttOptions::new(&config.node_id, &config.mqtt_host, config.mqtt_port.unwrap_or(1883));
	mqtt_options.set_credentials(config.mqtt_user.clone(), config.mqtt_pass.clone());
	let (mut mqtt_client, mut notifications) = Client::new(mqtt_options, 10);
	thread::spawn(move || {
		while let Ok(notification) = notifications.recv() {
			println!("{notification:?}");
		}
	});

	match reconnect_wrapper(&config, &mut mqtt_client) {
		Ok(()) => {}
		Err(err) => { panic!("{err:?}"); }
	}
}

fn get_device_id(device: &MonDevice) -> &str {
	match device {
		MonDevice::Sata { device, .. } => device,
		MonDevice::MdRaid { device, .. } => device,
	}
}

fn get_device_name(device: &MonDevice) -> &str {
	match device {
		MonDevice::Sata { name, .. } => name,
		MonDevice::MdRaid { name, .. } => name,
	}
}

fn get_state_topic(node_id: &str, device_id: &str) -> String {
	return format!("{}/{}/state", node_id, device_id);
}

fn get_attributes_topic(node_id: &str, device_id: &str) -> String {
	return format!("{}/{}/attributes", node_id, device_id);
}

fn reconnect_wrapper(config: &config_types::Config, client: &mut Client) -> Result<(), rumqttc::Error> {
	for device in config.devices.iter() {
		let device_id = get_device_id(device);
		let device_name = get_device_name(&device);
		client.publish(
			format!(
				"{}/sensor/{}/{}/config",
				config.discovery_prefix.as_ref().map(|dp| dp.as_str()).unwrap_or("homeassistant"),
				config.node_id,
				get_device_id(device),
			),
			QoS::AtLeastOnce,
			true,
			serde_json::to_vec(&json!({
				"icon": "mdi:harddisk",
				"name": device_name,
				"state_topic": get_state_topic(&config.node_id, &device_id),
				"json_attributes_topic": get_attributes_topic(&config.node_id, &device_id),
				"unique_id": device_id,
			})).unwrap(),
		).expect("Failed to push discovery message");
	}
	loop {
		for device in config.devices.iter() {
			let (device, result) = match device {
				MonDevice::Sata { device, .. } => (device, publish_sata(&config.node_id, device, client)),
				MonDevice::MdRaid { device, .. } => (device, publish_raid(&config.node_id, device, client)),
			};
			if let Err(err) = result {
				eprintln!("Error checking {device}:\n{err:?}");
			}
		}
		thread::sleep(Duration::from_secs(config.interval));
	}
}

fn publish_sata(node_id: &str, device: &str, client: &mut Client) -> anyhow::Result<()> {
	let mut smartctl = Command::new("smartctl")
		.args(["-iaj", "--nocheck", "standby"])
		.arg(String::from("/dev/") + device)
		.stdin(Stdio::null())
		.stdout(Stdio::piped())
		.spawn()
		.context("Couldn't run smartctl")?;
	let smart_results: SmartMonResults = serde_json::from_reader(smartctl.stdout.as_mut().unwrap())
		.context("Couldn't parse smartctl output")?;
	smartctl.wait().context("Failed to wait for smartctl")?;

	client.publish(
		get_state_topic(node_id, device),
		QoS::AtLeastOnce,
		false,
		if smart_results.smart_status.passed {
			"Healthy"
		} else {
			"Not healthy"
		},
	).map_err(|err| anyhow!("{:?}", err))?;
	client.publish(
		get_attributes_topic(node_id, device),
		QoS::AtLeastOnce,
		false,
		serde_json::to_string(&smart_results).unwrap(),
	).map_err(|err| anyhow!("{:?}", err))?;

	println!("{}", serde_json::to_string_pretty(&smart_results).unwrap());
	return Ok(());
}
fn publish_raid(node_id: &str, device: &str, client: &mut Client) -> anyhow::Result<()> {
	let uuid: String = read_file(format!("/sys/class/block/{device}/md/uuid"))
		.context("Couldn't get uuid")?;
	let sync_action: String = read_file(format!("/sys/class/block/{device}/md/sync_action"))
		.context("Couldn't get sync_action")?;
	let mut sync_progress: String = read_file(format!("/sys/class/block/{device}/md/sync_completed"))
		.context("Couldn't get sync_completed")?;
	let degraded: usize = parse_file(format!("/sys/class/block/{device}/md/degraded"))
		.context("Couldn't parse degraded status")?;

	if let Some(sync_progress_per) = convert_percent(&sync_progress) {
		sync_progress = sync_progress_per;
	}

	client.publish(
		get_state_topic(node_id, device),
		QoS::AtLeastOnce,
		false,
		if degraded > 0 {
			"Degraded"
		} else {
			&sync_action
		},
	).map_err(|err| anyhow!("{:?}", err))?;
	client.publish(
		get_attributes_topic(node_id, device),
		QoS::AtLeastOnce,
		false,
		serde_json::to_string(&json!({
			"uuid": uuid.trim(),
			"sync_action": sync_action.trim(),
			"sync_progress": sync_progress.trim(),
			"degraded_by": degraded,
		})).unwrap(),
	).map_err(|err| anyhow!("{:?}", err))?;

	println!("{device} (uuid {uuid}) is {degraded} disks degraded and is {sync_action} with progress {sync_progress}.");
	return Ok(());
}

fn convert_percent(data: &str) -> Option<String> {
	let (count, out_of) = data.split_once("/")?;
	let per: f32 = count.trim().parse::<f32>().ok()? / out_of.trim().parse::<f32>().ok()?;
	return Some(format!("{:.2}%", per * 100f32));
}

fn read_file(file: impl AsRef<Path>) -> anyhow::Result<String> {
	let mut status = String::new();
	File::open(file)
		.context("Couldn't open degraded status")?
		.read_to_string(&mut status)?;
	return Ok(status);
}

fn parse_file<T: FromStr<Err = E>, E: std::error::Error + Send + Sync + 'static>(file: impl AsRef<Path>) -> anyhow::Result<T> {
	return Ok(read_file(file)?.trim().parse()?);
}
