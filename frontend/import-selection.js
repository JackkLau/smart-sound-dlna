// Shared selection rules for video and favorites import. A BVID/CID identifies
// an audio track; preview IDs identify the current native authorization to queue it.
export default class BilibiliSelection {
  constructor() {
    this.records = new Map();
  }
  get count() {
    return [...this.records.values()].reduce((n, r) => n + r.selected.size, 0);
  }
  clear() {
    this.records.clear();
  }
  clearSelection() {
    for (const r of this.records.values()) r.selected.clear();
  }
  register(preview, selectDefault = false) {
    const previous = this.records.get(preview.bvid);
    if (!previous && this.records.size >= 100) {
      const disposable = [...this.records].find(([, r]) => !r.selected.size);
      if (!disposable)
        throw new Error("已选择 100 个视频，请先下载或清空部分选择。");
      this.records.delete(disposable[0]);
    }
    const valid = new Set(preview.parts.map((p) => p.cid));
    const selected = new Set(
      [...(previous?.selected || [])].filter((cid) => valid.has(cid)),
    );
    const record = { preview, selected };
    this.records.set(preview.bvid, record);
    if (!previous && selectDefault) {
      const part = preview.parts.find((p) => p.page === preview.selected_page);
      if (part) this.set(preview.bvid, part.cid, true);
    }
    return record;
  }
  set(bvid, cid, checked) {
    const record = this.records.get(bvid);
    if (!record?.preview.parts.some((p) => p.cid === cid))
      throw new Error("音轨信息已变化，请重新解析。");
    if (checked && !record.selected.has(cid) && this.count >= 100)
      throw new Error("每批最多选择 100 首音轨。");
    if (checked) record.selected.add(cid);
    else record.selected.delete(cid);
  }
  setAll(bvid, checked) {
    const record = this.records.get(bvid);
    if (!record) return;
    const cids = new Set(record.preview.parts.map((p) => p.cid));
    if (checked && this.count - record.selected.size + cids.size > 100)
      throw new Error("全选后会超过 100 首，请逐首勾选。");
    record.selected = checked ? cids : new Set();
  }
  payload() {
    return [...this.records.values()]
      .filter((r) => r.selected.size)
      .map((r) => ({ preview_id: r.preview.id, cids: [...r.selected] }));
  }
  accept(result) {
    for (const item of result.items) {
      if (!item.job) continue; // Skipped/failed tracks stay selected for correction.
      for (const r of this.records.values()) {
        if (r.preview.id === item.preview_id) r.selected.delete(item.cid);
      }
    }
  }
}
