// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//
// Copyright (c) 2024-2026 TripleACS Pty Ltd t/a 2pi Software

interface PostAcceptDialogProps {
  open: boolean;
  peerName: string;
  onSendNow: () => void;
  onLater: () => void;
}

export function PostAcceptDialog({ open, peerName, onSendNow, onLater }: PostAcceptDialogProps) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-70">
      <div className="bg-background border border-border rounded-lg p-6 max-w-sm w-full shadow-xl">
        <h2 className="text-lg font-semibold mb-2">Peer accepted</h2>
        <p className="text-sm text-muted-foreground mb-6">
          <strong>{peerName}</strong> has been added as a peer.
          Send them the workspace snapshot now so they can join?
        </p>
        <div className="flex justify-end gap-3">
          <button
            onClick={onLater}
            className="px-4 py-2 bg-secondary text-foreground rounded-md hover:bg-secondary/80"
          >
            Later
          </button>
          <button
            onClick={onSendNow}
            className="px-4 py-2 bg-primary text-primary-foreground rounded-md hover:opacity-90"
          >
            Send Snapshot
          </button>
        </div>
      </div>
    </div>
  );
}
