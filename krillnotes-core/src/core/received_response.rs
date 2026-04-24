// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

use super::error::KrillnotesError;

type Result<T> = std::result::Result<T, KrillnotesError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReceivedResponseStatus {
    Pending,
    PeerAdded,
    PermissionPending,
    SnapshotSent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceivedResponse {
    pub response_id: Uuid,
    pub invite_id: Uuid,
    pub workspace_id: String,
    pub workspace_name: String,
    pub invitee_public_key: String,
    pub invitee_declared_name: String,
    pub received_at: DateTime<Utc>,
    pub status: ReceivedResponseStatus,
    #[serde(default)]
    pub scope_note_id: Option<String>,
    #[serde(default)]
    pub scope_note_title: Option<String>,
    #[serde(default)]
    pub offered_role: String,
    #[serde(default)]
    pub response_channel: String, // "relay" | "file"
    #[serde(default)]
    pub relay_account_id: Option<String>,
}

impl ReceivedResponse {
    pub fn new(
        invite_id: Uuid,
        workspace_id: String,
        workspace_name: String,
        invitee_public_key: String,
        invitee_declared_name: String,
        scope_note_id: Option<String>,
        scope_note_title: Option<String>,
    ) -> Self {
        Self {
            response_id: Uuid::new_v4(),
            invite_id,
            workspace_id,
            workspace_name,
            invitee_public_key,
            invitee_declared_name,
            received_at: Utc::now(),
            status: ReceivedResponseStatus::Pending,
            scope_note_id,
            scope_note_title,
            offered_role: String::new(),
            response_channel: String::new(),
            relay_account_id: None,
        }
    }
}

pub struct ReceivedResponseManager {
    dir: PathBuf,
}

impl ReceivedResponseManager {
    pub fn new(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    pub fn save(&mut self, response: &ReceivedResponse) -> Result<()> {
        let path = self.path_for(response.response_id);
        let json = serde_json::to_string_pretty(response)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn get(&self, response_id: Uuid) -> Result<Option<ReceivedResponse>> {
        let path = self.path_for(response_id);
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(path)?;
        let record: ReceivedResponse = serde_json::from_str(&data)?;
        Ok(Some(record))
    }

    pub fn list(&self) -> Result<Vec<ReceivedResponse>> {
        let mut records = Vec::new();
        if !self.dir.exists() {
            return Ok(records);
        }
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let data = std::fs::read_to_string(&path)?;
                if let Ok(record) = serde_json::from_str::<ReceivedResponse>(&data) {
                    records.push(record);
                }
            }
        }
        records.sort_by(|a, b| b.received_at.cmp(&a.received_at));
        Ok(records)
    }

    pub fn list_by_workspace(&self, workspace_id: &str) -> Result<Vec<ReceivedResponse>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|r| r.workspace_id == workspace_id)
            .collect())
    }

    pub fn find_by_invite_and_invitee(
        &self,
        invite_id: Uuid,
        invitee_public_key: &str,
    ) -> Result<Option<ReceivedResponse>> {
        Ok(self
            .list()?
            .into_iter()
            .find(|r| r.invite_id == invite_id && r.invitee_public_key == invitee_public_key))
    }

    pub fn update_status(
        &mut self,
        response_id: Uuid,
        status: ReceivedResponseStatus,
    ) -> Result<()> {
        let mut record = self.get(response_id)?.ok_or_else(|| {
            KrillnotesError::Swarm(format!("Received response {response_id} not found"))
        })?;
        record.status = status;
        self.save(&record)
    }

    pub fn delete(&mut self, response_id: Uuid) -> Result<()> {
        let path = self.path_for(response_id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "received_response_tests.rs"]
mod tests;
