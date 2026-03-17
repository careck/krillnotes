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
pub enum AcceptedInviteStatus {
    WaitingSnapshot,
    WorkspaceCreated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedInvite {
    pub invite_id: Uuid,
    pub workspace_id: String,
    pub workspace_name: String,
    pub inviter_public_key: String,
    pub inviter_declared_name: String,
    pub accepted_at: DateTime<Utc>,
    pub response_relay_url: Option<String>,
    pub status: AcceptedInviteStatus,
    pub workspace_path: Option<String>,
    #[serde(default)]
    pub snapshot_path: Option<String>,
}

impl AcceptedInvite {
    pub fn new(
        invite_id: Uuid,
        workspace_id: String,
        workspace_name: String,
        inviter_public_key: String,
        inviter_declared_name: String,
        response_relay_url: Option<String>,
    ) -> Self {
        Self {
            invite_id,
            workspace_id,
            workspace_name,
            inviter_public_key,
            inviter_declared_name,
            accepted_at: Utc::now(),
            response_relay_url,
            status: AcceptedInviteStatus::WaitingSnapshot,
            workspace_path: None,
            snapshot_path: None,
        }
    }
}

pub struct AcceptedInviteManager {
    dir: PathBuf,
}

impl AcceptedInviteManager {
    pub fn new(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn path_for(&self, id: Uuid) -> PathBuf {
        self.dir.join(format!("{}.json", id))
    }

    pub fn save(&mut self, invite: &AcceptedInvite) -> Result<()> {
        let path = self.path_for(invite.invite_id);
        let json = serde_json::to_string_pretty(invite)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn get(&self, invite_id: Uuid) -> Result<Option<AcceptedInvite>> {
        let path = self.path_for(invite_id);
        if !path.exists() {
            return Ok(None);
        }
        let data = std::fs::read_to_string(path)?;
        let record: AcceptedInvite = serde_json::from_str(&data)?;
        Ok(Some(record))
    }

    pub fn list(&self) -> Result<Vec<AcceptedInvite>> {
        let mut records = Vec::new();
        if !self.dir.exists() {
            return Ok(records);
        }
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "json").unwrap_or(false) {
                let data = std::fs::read_to_string(&path)?;
                if let Ok(record) = serde_json::from_str::<AcceptedInvite>(&data) {
                    records.push(record);
                }
            }
        }
        records.sort_by(|a, b| b.accepted_at.cmp(&a.accepted_at));
        Ok(records)
    }

    pub fn list_waiting_snapshot(&self) -> Result<Vec<AcceptedInvite>> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|i| i.status == AcceptedInviteStatus::WaitingSnapshot)
            .collect())
    }

    pub fn update_status(
        &mut self,
        invite_id: Uuid,
        status: AcceptedInviteStatus,
        workspace_path: Option<String>,
    ) -> Result<()> {
        let mut record = self.get(invite_id)?.ok_or_else(|| {
            KrillnotesError::Swarm(format!("Accepted invite {invite_id} not found"))
        })?;
        record.status = status;
        if workspace_path.is_some() {
            record.workspace_path = workspace_path;
        }
        self.save(&record)
    }

    pub fn update_snapshot_path(&mut self, invite_id: Uuid, snapshot_path: String) -> Result<()> {
        let mut record = self.get(invite_id)?.ok_or_else(|| {
            KrillnotesError::Swarm(format!("Accepted invite {invite_id} not found"))
        })?;
        record.snapshot_path = Some(snapshot_path);
        self.save(&record)
    }

    pub fn delete(&mut self, invite_id: Uuid) -> Result<()> {
        let path = self.path_for(invite_id);
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "accepted_invite_tests.rs"]
mod tests;
