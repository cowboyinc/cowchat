use clap::ValueEnum;
use cowchat_client::CowchatClient;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

const SOFTWARE_DELIVERY_TEMPLATE: &str =
    include_str!("../../../templates/workflows/software-delivery/workflow.toml");

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub(crate) enum WorkflowTemplate {
    SoftwareDelivery,
}

impl WorkflowTemplate {
    fn contents(self) -> &'static str {
        match self {
            Self::SoftwareDelivery => SOFTWARE_DELIVERY_TEMPLATE,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct WorkflowHeader {
    name: String,
    version: u32,
}

#[derive(Debug, Deserialize, Serialize)]
struct Channel {
    description: String,
    room: String,
    events: Vec<String>,
    use_when: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WorkflowFile {
    workflow: WorkflowHeader,
    channels: BTreeMap<String, Channel>,
}

#[derive(Debug, Serialize)]
struct WorkflowChannelsOutput<'a> {
    workflow: &'a WorkflowHeader,
    channels: Vec<ChannelCard<'a>>,
}

#[derive(Debug, Serialize)]
struct ChannelCard<'a> {
    id: &'a str,
    description: &'a str,
    room: &'a str,
    events: &'a [String],
    use_when: &'a [String],
}

#[derive(Debug, Serialize)]
struct WorkflowSyncOutput {
    workflow: WorkflowHeader,
    channels: Vec<WorkflowSyncChannel>,
}

#[derive(Debug, Serialize)]
struct WorkflowSyncChannel {
    id: String,
    room: String,
    room_id: String,
    action: &'static str,
}

pub(crate) fn init(
    template: WorkflowTemplate,
    path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() {
        return Err(format!(
            "workflow already exists at {}; refusing to overwrite it",
            path.display()
        )
        .into());
    }

    let parent = path
        .parent()
        .ok_or("workflow path must have a parent directory")?;
    fs::create_dir_all(parent)?;

    let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
    file.write_all(template.contents().as_bytes())?;
    file.sync_all()?;

    println!(
        "Initialized {} workflow at {}",
        template_name(template),
        path.display()
    );
    println!("Inspect its channels with: cowchat workflow channels --json");
    Ok(())
}

pub(crate) fn render_channels(
    path: &Path,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let workflow = load(path)?;
    let output = WorkflowChannelsOutput {
        workflow: &workflow.workflow,
        channels: workflow
            .channels
            .iter()
            .map(|(id, channel)| ChannelCard {
                id,
                description: &channel.description,
                room: &channel.room,
                events: &channel.events,
                use_when: &channel.use_when,
            })
            .collect(),
    };

    if json {
        return Ok(format!("{}\n", serde_json::to_string(&output)?));
    }

    let mut lines = vec![format!(
        "{} v{} workflow channels:",
        output.workflow.name, output.workflow.version
    )];
    for channel in output.channels {
        lines.push(format!(
            "\n{} ({})\n  {}\n  Use when: {}\n  Events: {}",
            channel.id,
            channel.room,
            channel.description,
            channel.use_when.join("; "),
            channel.events.join(", ")
        ));
    }
    Ok(format!("{}\n", lines.join("\n")))
}

pub(crate) async fn sync_rooms(
    path: &Path,
    client: &CowchatClient,
    json: bool,
) -> Result<String, Box<dyn std::error::Error>> {
    let workflow = load(path)?;
    let existing_rooms = client.list_rooms(None).await?;
    let mut synced = Vec::with_capacity(workflow.channels.len());

    for (id, channel) in &workflow.channels {
        let existing = existing_rooms.iter().find(|room| room.name == channel.room);
        let (room_id, action) = match existing {
            Some(room) => (room.room_id.clone(), "existing"),
            None => {
                let room = client
                    .create_room(&channel.room, Some(&channel.description), None)
                    .await?;
                (room.room_id, "created")
            }
        };
        synced.push(WorkflowSyncChannel {
            id: id.clone(),
            room: channel.room.clone(),
            room_id,
            action,
        });
    }

    let output = WorkflowSyncOutput {
        workflow: workflow.workflow,
        channels: synced,
    };
    if json {
        return Ok(format!("{}\n", serde_json::to_string(&output)?));
    }

    let mut lines = vec![format!(
        "Synchronized {} v{} workflow:",
        output.workflow.name, output.workflow.version
    )];
    for channel in output.channels {
        lines.push(format!(
            "  {}: {} ({}) [{}]",
            channel.id, channel.room, channel.room_id, channel.action
        ));
    }
    Ok(format!("{}\n", lines.join("\n")))
}

fn load(path: &Path) -> Result<WorkflowFile, Box<dyn std::error::Error>> {
    let contents = fs::read_to_string(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("could not read workflow at {}: {error}", path.display()),
        )
    })?;
    Ok(toml::from_str(&contents)
        .map_err(|error| format!("invalid workflow TOML at {}: {error}", path.display()))?)
}

fn template_name(template: WorkflowTemplate) -> &'static str {
    match template {
        WorkflowTemplate::SoftwareDelivery => "software-delivery",
    }
}

#[cfg(test)]
mod tests {
    use super::{init, render_channels, WorkflowTemplate};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn embedded_software_delivery_template_has_the_expected_channel_cards() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".cowchat/workflow.toml");
        init(WorkflowTemplate::SoftwareDelivery, &path).unwrap();

        let output = render_channels(&path, true).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        assert_eq!(parsed["workflow"]["name"], "software-delivery");
        let ids = parsed["channels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|channel| channel["id"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["decisions", "dispatch", "handoffs", "review"]);
    }

    #[test]
    fn init_refuses_to_overwrite_an_existing_workflow() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("workflow.toml");
        fs::write(&path, "user-owned workflow\n").unwrap();

        let error = init(WorkflowTemplate::SoftwareDelivery, &path).unwrap_err();
        assert!(error.to_string().contains("refusing to overwrite"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "user-owned workflow\n");
    }
}
