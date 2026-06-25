// Revision guard for cross-window state sync (WB-14, 18 §5/§6). Backend-authoritative state
// changes carry a monotonically increasing revision; a window applies an incoming update only
// when its revision is newer than what it has already applied. This drops stale/echoed events
// and prevents the two-way "store ↔ window" sync from looping across windows.
export class RevisionGate {
  private applied = Number.NEGATIVE_INFINITY;

  /** Record + accept `incoming` iff it is newer than the last applied revision. */
  accept(incoming: number): boolean {
    if (incoming > this.applied) {
      this.applied = incoming;
      return true;
    }
    return false;
  }

  /** The last applied revision (-1 before anything has been accepted). */
  get current(): number {
    return this.applied === Number.NEGATIVE_INFINITY ? -1 : this.applied;
  }

  reset(): void {
    this.applied = Number.NEGATIVE_INFINITY;
  }
}
