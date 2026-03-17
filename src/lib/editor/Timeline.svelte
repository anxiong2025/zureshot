<script>
  let {
    thumbnails = [],
    waveform = null,
    duration = 0,
    currentTime = 0,
    trimStart = 0,
    trimEnd = 0,
    zoomKeyframes = [],
    zoomEnabled = true,
    speedSegments = [],
    cutPoints = [],
    deletedSegments = new Set(),
    selectedClipIndex = -1,
    onSeek = () => {},
    onTrimChange = () => {},
    onZoomKeyframeChange = () => {},
    onSpeedSegmentChange = () => {},
    onSelectionChange = () => {},
    onAddZoomAtTime = () => {},
    onAddSpeedAtTime = () => {},
    onCutAtTime = () => {},
    onClipSelect = () => {},
    onClipDelete = () => {},
    onJoinClips = () => {},
  } = $props();

  let trackEl = $state(null);
  let zoomBarEl = $state(null);
  let speedBarEl = $state(null);
  let dragging = $state(null);
  let selected = $state(null);
  let hoverZoom = $state(null);
  let hoverSpeed = $state(null);

  // ─── Segments from cut points ───
  let allSegments = $derived.by(() => {
    const pts = [trimStart, ...cutPoints.filter(t => t > trimStart + 0.05 && t < trimEnd - 0.05).sort((a, b) => a - b), trimEnd];
    return pts.slice(0, -1).map((start, i) => ({
      index: i,
      start,
      end: pts[i + 1],
      dur: pts[i + 1] - start,
      deleted: deletedSegments.has(i),
    }));
  });

  // ─── Magnetic layout: kept clips snap together ───
  let clips = $derived.by(() => {
    const kept = allSegments.filter(s => !s.deleted);
    const totalDur = kept.reduce((sum, s) => sum + s.dur, 0);
    if (totalDur <= 0) return [];
    let pct = 0;
    return kept.map((s, i) => {
      const w = (s.dur / totalDur) * 100;
      const clip = { ...s, pctStart: pct, pctWidth: w, clipIdx: i };
      pct += w;
      return clip;
    });
  });

  let totalKeptDur = $derived(clips.reduce((sum, c) => sum + c.dur, 0));
  let hasMultipleClips = $derived(allSegments.length > 1);

  // ─── Time <-> Magnetic position mapping ───
  function timeToMagPct(time) {
    for (const c of clips) {
      if (time <= c.end + 0.001) {
        if (time < c.start) return c.pctStart;
        const within = c.dur > 0 ? (time - c.start) / c.dur : 0;
        return c.pctStart + Math.min(1, within) * c.pctWidth;
      }
    }
    return 100;
  }

  function magPctToTime(pct) {
    for (const c of clips) {
      const cEnd = c.pctStart + c.pctWidth;
      if (pct <= cEnd + 0.001) {
        if (pct < c.pctStart) return c.start;
        const within = c.pctWidth > 0 ? (pct - c.pctStart) / c.pctWidth : 0;
        return c.start + Math.min(1, within) * c.dur;
      }
    }
    const last = clips[clips.length - 1];
    return last ? last.end : trimEnd;
  }

  function xToMagPct(clientX, el) {
    const target = el || trackEl;
    if (!target) return 0;
    const rect = target.getBoundingClientRect();
    return Math.max(0, Math.min(100, ((clientX - rect.left) / rect.width) * 100));
  }

  let playheadPct = $derived(timeToMagPct(currentTime));

  // ─── Clip data helpers ───
  function getClipThumbs(clip) {
    return thumbnails.filter(t => t.time_secs >= clip.start - 0.05 && t.time_secs <= clip.end + 0.05);
  }

  function getClipWave(clip) {
    if (!waveform?.samples?.length) return [];
    const n = waveform.samples.length;
    const si = Math.floor((clip.start / duration) * n);
    const ei = Math.ceil((clip.end / duration) * n);
    return waveform.samples.slice(si, ei);
  }

  // ─── Join info between adjacent clips ───
  let joins = $derived.by(() => {
    const result = [];
    for (let i = 0; i < clips.length - 1; i++) {
      const cur = clips[i];
      const next = clips[i + 1];
      const hasGap = allSegments.some(s =>
        s.deleted && s.start >= cur.end - 0.01 && s.end <= next.start + 0.01
      );
      result.push({
        pct: cur.pctStart + cur.pctWidth,
        cutTime: cur.end,
        hasDeletedBetween: hasGap,
      });
    }
    return result;
  });

  // ─── Ruler marks ───
  let rulerMarks = $derived.by(() => {
    if (clips.length === 0) return [];
    const marks = [];
    marks.push({ time: clips[0].start, pct: 0 });
    const last = clips[clips.length - 1];
    marks.push({ time: last.end, pct: 100 });
    for (const c of clips) {
      if (c.dur < 2) continue;
      const interval = c.dur > 30 ? 10 : c.dur > 10 ? 5 : c.dur > 5 ? 2 : 1;
      const first = Math.ceil(c.start / interval) * interval;
      for (let t = first; t < c.end - 0.3; t += interval) {
        if (t > c.start + 0.3) {
          const within = (t - c.start) / c.dur;
          marks.push({ time: t, pct: c.pctStart + within * c.pctWidth });
        }
      }
    }
    marks.sort((a, b) => a.pct - b.pct);
    return marks.filter((m, i) => i === 0 || m.pct - marks[i - 1].pct > 4);
  });

  // ─── Zoom segments with magnetic positions ───
  const TRANSITION_IN = 0.55;
  const TRANSITION_OUT = 0.60;
  const DEFAULT_HOLD = 0.6;
  const MERGE_GAP = 1.0;

  let zoomSegments = $derived.by(() => {
    if (!zoomKeyframes || zoomKeyframes.length === 0 || !zoomEnabled) return [];
    const sorted = [...zoomKeyframes].sort((a, b) => a.time - b.time);
    const groups = [];
    let grp = { kfs: [sorted[0]], indices: [0] };
    for (let i = 1; i < sorted.length; i++) {
      const prev = grp.kfs[grp.kfs.length - 1];
      const prevEnd = prev.time + (prev.hold || DEFAULT_HOLD);
      if (sorted[i].time - prevEnd < MERGE_GAP) {
        grp.kfs.push(sorted[i]);
        grp.indices.push(i);
      } else {
        groups.push(grp);
        grp = { kfs: [sorted[i]], indices: [i] };
      }
    }
    groups.push(grp);
    return groups.map((g, gi) => {
      const first = g.kfs[0];
      const last = g.kfs[g.kfs.length - 1];
      const start = Math.max(0, first.time - TRANSITION_IN);
      const end = Math.min(duration, last.time + (last.hold || DEFAULT_HOLD) + TRANSITION_OUT);
      const maxZoom = Math.max(...g.kfs.map(k => k.zoom));
      return {
        id: gi, start, end, zoom: maxZoom,
        count: g.kfs.length, kfIndices: g.indices,
        magL: timeToMagPct(start), magR: timeToMagPct(end),
      };
    });
  });

  // ─── Track interactions ───
  function handleTrackClick(e) {
    if (dragging) return;
    selected = null;
    onSelectionChange(null);
    onClipSelect(-1);
    const pct = xToMagPct(e.clientX);
    onSeek(magPctToTime(pct));
  }

  function handleClipClick(clip, e) {
    e.stopPropagation();
    if (hasMultipleClips) onClipSelect(clip.index);
    selected = null;
    onSelectionChange(null);
    const pct = xToMagPct(e.clientX);
    onSeek(magPctToTime(pct));
  }

  function handleJoinClick(join, e) {
    e.stopPropagation();
    onJoinClips(join.cutTime);
  }

  function startDrag(type, e) {
    e.stopPropagation();
    e.preventDefault();
    dragging = type;
    const startX = e.clientX;
    const origTrimStart = trimStart;
    const origTrimEnd = trimEnd;

    const handleMove = (ev) => {
      if (type === 'playhead') {
        const pct = xToMagPct(ev.clientX);
        onSeek(magPctToTime(pct));
      } else {
        const rect = trackEl.getBoundingClientRect();
        const dx = ev.clientX - startX;
        const tpp = totalKeptDur > 0 ? totalKeptDur / rect.width : duration / rect.width;
        const dt = dx * tpp;
        if (type === 'trim-start') {
          onTrimChange(Math.max(0, Math.min(origTrimEnd - 0.1, origTrimStart + dt)), trimEnd);
        } else if (type === 'trim-end') {
          onTrimChange(trimStart, Math.max(trimStart + 0.1, Math.min(duration, origTrimEnd + dt)));
        }
      }
    };

    const handleUp = () => {
      dragging = null;
      window.removeEventListener('mousemove', handleMove);
      window.removeEventListener('mouseup', handleUp);
    };
    window.addEventListener('mousemove', handleMove);
    window.addEventListener('mouseup', handleUp);
  }

  // ─── Zoom segment interactions ───
  function selectZoomSeg(seg, e) {
    e.stopPropagation();
    selected = { type: 'zoom', index: seg.id };
    onSelectionChange({ type: 'zoom', index: seg.id, value: seg.zoom });
    onSeek(seg.start + (seg.end - seg.start) / 2);
  }

  function deleteZoomSeg() {
    if (!selected || selected.type !== 'zoom') return;
    const seg = zoomSegments[selected.index];
    if (!seg) return;
    const sorted = [...zoomKeyframes].sort((a, b) => a.time - b.time);
    const timesToRemove = new Set(seg.kfIndices.map(i => sorted[i]?.time));
    onZoomKeyframeChange(zoomKeyframes.filter(k => !timesToRemove.has(k.time)));
    selected = null;
    onSelectionChange(null);
  }

  function startZoomDrag(seg, edge, e) {
    e.stopPropagation();
    e.preventDefault();
    dragging = 'zoom-seg';
    const sorted = [...zoomKeyframes].sort((a, b) => a.time - b.time);
    const firstKfIdx = seg.kfIndices[0];
    const lastKfIdx = seg.kfIndices[seg.kfIndices.length - 1];

    const handleMove = (ev) => {
      const pct = xToMagPct(ev.clientX, zoomBarEl);
      const time = magPctToTime(pct);
      const updated = [...zoomKeyframes];
      if (edge === 'left' && sorted[firstKfIdx]) {
        const origIdx = zoomKeyframes.findIndex(k => k.time === sorted[firstKfIdx].time);
        if (origIdx >= 0) {
          updated[origIdx] = { ...updated[origIdx], time: Math.max(0, Math.min(sorted[firstKfIdx].time + 0.5, time + TRANSITION_IN)) };
        }
      } else if (edge === 'right' && sorted[lastKfIdx]) {
        const origIdx = zoomKeyframes.findIndex(k => k.time === sorted[lastKfIdx].time);
        if (origIdx >= 0) {
          const hold = sorted[lastKfIdx].hold || DEFAULT_HOLD;
          updated[origIdx] = { ...updated[origIdx], time: Math.max(0, time - hold - TRANSITION_OUT) };
        }
      }
      onZoomKeyframeChange(updated);
    };

    const handleUp = () => {
      dragging = null;
      window.removeEventListener('mousemove', handleMove);
      window.removeEventListener('mouseup', handleUp);
    };
    window.addEventListener('mousemove', handleMove);
    window.addEventListener('mouseup', handleUp);
  }

  // ─── Speed segment interactions ───
  let draggingSpeed = $state(null);

  function selectSpeedSeg(seg, e) {
    e.stopPropagation();
    if (draggingSpeed) return;
    selected = { type: 'speed', id: seg.id };
    onSelectionChange({ type: 'speed', id: seg.id, value: seg.speed });
    onSeek(seg.start + (seg.end - seg.start) / 2);
  }

  function deleteSpeedSeg() {
    if (!selected || selected.type !== 'speed') return;
    onSpeedSegmentChange(speedSegments.filter(s => s.id !== selected.id));
    selected = null;
    onSelectionChange(null);
  }

  function startSpeedDrag(seg, edge, e) {
    e.stopPropagation();
    e.preventDefault();
    draggingSpeed = { id: seg.id, edge };

    const handleMove = (ev) => {
      const pct = xToMagPct(ev.clientX, speedBarEl);
      const time = magPctToTime(pct);
      const updated = speedSegments.map(s => {
        if (s.id !== seg.id) return s;
        if (edge === 'left') return { ...s, start: Math.max(0, Math.min(s.end - 0.2, time)) };
        if (edge === 'right') return { ...s, end: Math.max(s.start + 0.2, Math.min(duration, time)) };
        const w = s.end - s.start;
        const ns = Math.max(0, Math.min(duration - w, time - w / 2));
        return { ...s, start: ns, end: ns + w };
      });
      onSpeedSegmentChange(updated);
    };

    const handleUp = () => {
      draggingSpeed = null;
      window.removeEventListener('mousemove', handleMove);
      window.removeEventListener('mouseup', handleUp);
    };
    window.addEventListener('mousemove', handleMove);
    window.addEventListener('mouseup', handleUp);
  }

  // ─── Hover for add buttons ───
  function handleZoomBarMove(e) {
    if (dragging) { hoverZoom = null; return; }
    const pct = xToMagPct(e.clientX, zoomBarEl);
    const time = magPctToTime(pct);
    if (zoomSegments.some(s => time >= s.start && time <= s.end)) { hoverZoom = null; return; }
    hoverZoom = { x: pct, time };
  }

  function handleSpeedBarMove(e) {
    if (dragging || draggingSpeed) { hoverSpeed = null; return; }
    const pct = xToMagPct(e.clientX, speedBarEl);
    const time = magPctToTime(pct);
    if (speedSegments.some(s => time >= s.start && time <= s.end)) { hoverSpeed = null; return; }
    hoverSpeed = { x: pct, time };
  }

  function addZoomHere(e) {
    e.stopPropagation();
    if (!hoverZoom) return;
    onAddZoomAtTime(hoverZoom.time, 2.0);
    hoverZoom = null;
  }

  function addSpeedHere(e) {
    e.stopPropagation();
    if (!hoverSpeed) return;
    onAddSpeedAtTime(hoverSpeed.time, 0.5);
    hoverSpeed = null;
  }

  function handleSubBarClick(e) {
    if (e.target === e.currentTarget) {
      selected = null;
      onSelectionChange(null);
    }
  }

  function formatTimeShort(s) {
    const m = Math.floor(s / 60);
    const sec = Math.floor(s % 60);
    return `${m}:${sec.toString().padStart(2, '0')}`;
  }
</script>

<div class="timeline">
  <!-- Ruler -->
  <div class="ruler">
    {#each rulerMarks as mark}
      <span class="ruler-mark" style="left: {mark.pct}%">{formatTimeShort(mark.time)}</span>
    {/each}
  </div>

  <!-- Magnetic Track -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="track" bind:this={trackEl} onmousedown={handleTrackClick}>
    {#each clips as clip (clip.index)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="clip"
        class:selected={selectedClipIndex === clip.index}
        class:first={clip.clipIdx === 0}
        class:last={clip.clipIdx === clips.length - 1}
        style="left: {clip.pctStart}%; width: {clip.pctWidth}%;"
        onclick={(e) => handleClipClick(clip, e)}
      >
        <div class="clip-thumbs">
          {#each getClipThumbs(clip) as thumb}
            {@const pos = clip.dur > 0 ? ((thumb.time_secs - clip.start) / clip.dur) * 100 : 0}
            <img src={thumb.data_url} alt="" class="clip-thumb-img" style="left: {pos}%" draggable="false" />
          {/each}
        </div>

        {#if waveform?.samples?.length}
          {@const bars = getClipWave(clip)}
          <div class="clip-waveform">
            {#each bars as amp, i}
              <div class="clip-wave-bar" style="height: {Math.max(2, amp * 100)}%; left: {(i / bars.length) * 100}%; width: {100 / bars.length}%"></div>
            {/each}
          </div>
        {/if}

        {#if clip.dur >= 0.5}
          <span class="clip-dur">{formatTimeShort(clip.dur)}</span>
        {/if}
      </div>
    {/each}

    <!-- Join lines -->
    {#each joins as join}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="join-line"
        class:has-gap={join.hasDeletedBetween}
        style="left: {join.pct}%"
        onclick={(e) => handleJoinClick(join, e)}
        title={join.hasDeletedBetween ? 'Removed clips here' : 'Click to rejoin'}
      >
        {#if join.hasDeletedBetween}
          <div class="join-gap-dot"></div>
        {/if}
      </div>
    {/each}

    <!-- Trim handles -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="trim-handle trim-start" class:trimmed={trimStart > 0.05} onmousedown={(e) => startDrag('trim-start', e)}>
      <div class="trim-grip"><div class="trim-ln"></div><div class="trim-ln"></div></div>
    </div>
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="trim-handle trim-end" class:trimmed={trimEnd < duration - 0.05} onmousedown={(e) => startDrag('trim-end', e)}>
      <div class="trim-grip"><div class="trim-ln"></div><div class="trim-ln"></div></div>
    </div>

    <!-- Playhead -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="playhead" style="left: {playheadPct}%" onmousedown={(e) => startDrag('playhead', e)}>
      <div class="playhead-head"></div>
      <div class="playhead-line"></div>
    </div>
  </div>

  <!-- Zoom sub-track -->
  <div class="sub-track zoom-sub">
    <span class="sub-label">
      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
    </span>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="sub-bar" bind:this={zoomBarEl} onclick={handleSubBarClick} onmousemove={handleZoomBarMove} onmouseleave={() => hoverZoom = null}>
      {#each zoomSegments as seg}
        {@const left = seg.magL}
        {@const width = Math.max(0, seg.magR - seg.magL)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="segment zoom-seg"
          class:selected={selected?.type === 'zoom' && selected.index === seg.id}
          style="left: {left}%; width: {width}%"
          onclick={(e) => selectZoomSeg(seg, e)}
        >
          <div class="seg-edge seg-edge-l" onmousedown={(e) => startZoomDrag(seg, 'left', e)}></div>
          <span class="seg-text">{seg.zoom.toFixed(1)}x</span>
          <button class="seg-remove" onclick={(e) => { e.stopPropagation(); selected = { type: 'zoom', index: seg.id }; deleteZoomSeg(); }}>&#xd7;</button>
          <div class="seg-edge seg-edge-r" onmousedown={(e) => startZoomDrag(seg, 'right', e)}></div>
        </div>
      {/each}
      <div class="sub-playhead" style="left: {playheadPct}%"></div>
      {#if hoverZoom}
        <button class="add-seg-btn zoom-add" style="left: {hoverZoom.x}%" onclick={addZoomHere} title="Add zoom">
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
        </button>
      {/if}
    </div>
  </div>

  <!-- Speed sub-track -->
  <div class="sub-track speed-sub">
    <span class="sub-label">
      <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
    </span>
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="sub-bar" bind:this={speedBarEl} onclick={handleSubBarClick} onmousemove={handleSpeedBarMove} onmouseleave={() => hoverSpeed = null}>
      {#each speedSegments as seg}
        {@const magL = timeToMagPct(seg.start)}
        {@const magR = timeToMagPct(seg.end)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="segment speed-seg"
          class:selected={selected?.type === 'speed' && selected.id === seg.id}
          style="left: {magL}%; width: {Math.max(0, magR - magL)}%"
          onclick={(e) => selectSpeedSeg(seg, e)}
          onmousedown={(e) => { if (e.target.classList.contains('speed-seg')) startSpeedDrag(seg, 'body', e); }}
        >
          <div class="seg-edge seg-edge-l" onmousedown={(e) => startSpeedDrag(seg, 'left', e)}></div>
          <span class="seg-text">{seg.speed}x</span>
          <button class="seg-remove" onclick={(e) => { e.stopPropagation(); selected = { type: 'speed', id: seg.id }; deleteSpeedSeg(); }}>&#xd7;</button>
          <div class="seg-edge seg-edge-r" onmousedown={(e) => startSpeedDrag(seg, 'right', e)}></div>
        </div>
      {/each}
      <div class="sub-playhead" style="left: {playheadPct}%"></div>
      {#if hoverSpeed}
        <button class="add-seg-btn speed-add" style="left: {hoverSpeed.x}%" onclick={addSpeedHere} title="Add speed">
          <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
        </button>
      {/if}
    </div>
  </div>
</div>

<style>
  .timeline {
    padding: 8px 16px 12px;
    user-select: none;
  }

  .ruler {
    position: relative;
    height: 18px;
    margin-bottom: 4px;
  }

  .ruler-mark {
    position: absolute;
    transform: translateX(-50%);
    font-size: 9px;
    font-weight: 500;
    color: #555;
    font-variant-numeric: tabular-nums;
  }

  /* ─── Magnetic Track ─── */
  .track {
    position: relative;
    height: 64px;
    border-radius: 6px;
    background: #111114;
    cursor: pointer;
  }

  /* ─── Clip Blocks ─── */
  .clip {
    position: absolute;
    top: 0;
    bottom: 0;
    overflow: hidden;
    background: #1a1a20;
    border: 1.5px solid rgba(255,255,255,0.06);
    cursor: pointer;
    z-index: 1;
    transition: left 0.3s cubic-bezier(0.22, 0.68, 0.35, 1.0),
                width 0.3s cubic-bezier(0.22, 0.68, 0.35, 1.0),
                border-color 0.15s, box-shadow 0.15s;
  }

  .clip::before {
    content: '';
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 2px;
    background: linear-gradient(90deg, rgba(108,92,231,0.4), rgba(108,92,231,0.12));
    z-index: 2;
  }

  .clip.first { border-radius: 6px 0 0 6px; }
  .clip.last { border-radius: 0 6px 6px 0; }
  .clip.first.last { border-radius: 6px; }

  .clip:hover {
    border-color: rgba(108, 92, 231, 0.35);
  }

  .clip.selected {
    border-color: #6c5ce7;
    box-shadow: inset 0 0 0 1px rgba(108, 92, 231, 0.2), 0 0 12px rgba(108, 92, 231, 0.15);
    z-index: 2;
  }

  .clip-thumbs {
    position: absolute;
    inset: 0;
  }

  .clip-thumb-img {
    position: absolute;
    top: 0;
    height: 100%;
    width: auto;
    transform: translateX(-50%);
    opacity: 0.55;
    pointer-events: none;
  }

  .clip-waveform {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    height: 18px;
    display: flex;
    align-items: flex-end;
    pointer-events: none;
  }

  .clip-wave-bar {
    position: absolute;
    bottom: 0;
    background: rgba(108, 92, 231, 0.35);
    min-height: 1px;
    border-radius: 1px 1px 0 0;
  }

  .clip-dur {
    position: absolute;
    bottom: 3px;
    right: 6px;
    font-size: 9px;
    font-weight: 600;
    color: rgba(255,255,255,0.35);
    pointer-events: none;
    font-variant-numeric: tabular-nums;
    text-shadow: 0 1px 3px rgba(0,0,0,0.8);
  }

  /* ─── Join Lines ─── */
  .join-line {
    position: absolute;
    top: 2px;
    bottom: 2px;
    width: 7px;
    transform: translateX(-3.5px);
    cursor: pointer;
    z-index: 3;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: left 0.3s cubic-bezier(0.22, 0.68, 0.35, 1.0);
  }

  .join-line::before {
    content: '';
    position: absolute;
    top: 2px;
    bottom: 2px;
    width: 1px;
    background: rgba(255,255,255,0.1);
    transition: background 0.15s, box-shadow 0.15s;
  }

  .join-line:hover::before {
    background: rgba(108, 92, 231, 0.6);
    box-shadow: 0 0 6px rgba(108, 92, 231, 0.4);
  }

  .join-line.has-gap::before {
    background: rgba(231, 76, 60, 0.35);
    width: 2px;
  }

  .join-line.has-gap:hover::before {
    background: rgba(231, 76, 60, 0.7);
    box-shadow: 0 0 6px rgba(231, 76, 60, 0.4);
  }

  .join-gap-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: rgba(231, 76, 60, 0.6);
    z-index: 1;
    pointer-events: none;
  }

  /* ─── Trim Handles ─── */
  .trim-handle {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 10px;
    cursor: col-resize;
    z-index: 5;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0;
    transition: opacity 0.15s;
  }

  .track:hover .trim-handle { opacity: 0.4; }
  .trim-handle:hover { opacity: 1 !important; }
  .trim-handle.trimmed { opacity: 1; }

  .trim-start {
    left: 0;
    border-radius: 6px 0 0 6px;
    background: #f39c12;
  }

  .trim-end {
    right: 0;
    border-radius: 0 6px 6px 0;
    background: #f39c12;
  }

  .trim-grip { display: flex; gap: 2px; }
  .trim-ln { width: 1.5px; height: 16px; background: rgba(0,0,0,0.4); border-radius: 1px; }

  /* ─── Playhead ─── */
  .playhead {
    position: absolute;
    top: -4px;
    bottom: 0;
    width: 2px;
    transform: translateX(-1px);
    z-index: 10;
    cursor: col-resize;
    pointer-events: auto;
  }

  .playhead-head {
    width: 10px;
    height: 10px;
    background: #fff;
    border-radius: 2px;
    transform: translateX(-4px) rotate(45deg);
    position: relative;
    top: -2px;
    box-shadow: 0 1px 4px rgba(0,0,0,0.5);
  }

  .playhead-line {
    width: 2px;
    position: absolute;
    top: 6px;
    bottom: 0;
    background: #fff;
    box-shadow: 0 0 6px rgba(255,255,255,0.3);
  }

  /* ─── Sub-tracks ─── */
  .sub-track {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 4px;
  }

  .sub-label {
    width: 16px;
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    opacity: 0.5;
  }

  .sub-bar {
    flex: 1;
    position: relative;
    height: 24px;
    border-radius: 4px;
    background: rgba(255,255,255,0.02);
    overflow: visible;
  }

  .segment {
    position: absolute;
    top: 2px;
    bottom: 2px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
    min-width: 28px;
    cursor: pointer;
    transition: left 0.3s cubic-bezier(0.22, 0.68, 0.35, 1.0),
                width 0.3s cubic-bezier(0.22, 0.68, 0.35, 1.0),
                border-color 0.15s, background 0.15s;
  }

  .zoom-seg {
    background: rgba(0, 184, 148, 0.15);
    border: 1px solid rgba(0, 184, 148, 0.3);
  }
  .zoom-seg:hover {
    background: rgba(0, 184, 148, 0.25);
    border-color: rgba(0, 184, 148, 0.6);
  }
  .zoom-seg.selected {
    background: rgba(0, 184, 148, 0.3);
    border-color: #00b894;
    box-shadow: 0 0 8px rgba(0, 184, 148, 0.3);
  }

  .speed-seg {
    background: rgba(243, 156, 18, 0.15);
    border: 1px solid rgba(243, 156, 18, 0.3);
    cursor: grab;
  }
  .speed-seg:hover {
    background: rgba(243, 156, 18, 0.25);
    border-color: rgba(243, 156, 18, 0.6);
  }
  .speed-seg:active { cursor: grabbing; }
  .speed-seg.selected {
    background: rgba(243, 156, 18, 0.3);
    border-color: #f39c12;
    box-shadow: 0 0 8px rgba(243, 156, 18, 0.3);
  }

  .seg-text {
    font-size: 9px;
    font-weight: 700;
    pointer-events: none;
  }
  .zoom-seg .seg-text { color: #00b894; }
  .zoom-seg.selected .seg-text { color: #55efc4; }
  .speed-seg .seg-text { color: #f39c12; }
  .speed-seg.selected .seg-text { color: #fdcb6e; }

  .seg-edge {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 6px;
    cursor: col-resize;
    z-index: 2;
  }
  .seg-edge-l { left: 0; border-radius: 4px 0 0 4px; }
  .seg-edge-r { right: 0; border-radius: 0 4px 4px 0; }
  .seg-edge:hover { background: rgba(255,255,255,0.1); }

  .seg-remove {
    position: absolute;
    right: 2px;
    top: 50%;
    transform: translateY(-50%);
    width: 14px;
    height: 14px;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: rgba(255,255,255,0.3);
    font-size: 11px;
    font-weight: 700;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0;
    opacity: 0;
    transition: opacity 0.15s;
  }
  .segment:hover .seg-remove { opacity: 1; }
  .seg-remove:hover { background: rgba(231,76,60,0.2); color: #e74c3c; }

  .sub-playhead {
    position: absolute;
    top: 0;
    bottom: 0;
    width: 1px;
    background: rgba(255,255,255,0.5);
    pointer-events: none;
    z-index: 5;
  }

  .zoom-sub .sub-label { color: #00b894; }
  .speed-sub .sub-label { color: #f39c12; }

  /* ─── Add Button ─── */
  .add-seg-btn {
    position: absolute;
    top: 50%;
    transform: translate(-50%, -50%);
    width: 22px;
    height: 22px;
    border: 1.5px solid rgba(255,255,255,0.2);
    border-radius: 5px;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    z-index: 8;
    padding: 0;
    transition: all 0.15s;
    pointer-events: auto;
  }

  .zoom-add {
    background: rgba(0,184,148,0.25);
    border-color: rgba(0,184,148,0.5);
    color: #55efc4;
  }
  .zoom-add:hover {
    background: rgba(0,184,148,0.5);
    border-color: #00b894;
    box-shadow: 0 0 8px rgba(0,184,148,0.4);
  }

  .speed-add {
    background: rgba(243,156,18,0.25);
    border-color: rgba(243,156,18,0.5);
    color: #fdcb6e;
  }
  .speed-add:hover {
    background: rgba(243,156,18,0.5);
    border-color: #f39c12;
    box-shadow: 0 0 8px rgba(243,156,18,0.4);
  }
</style>
