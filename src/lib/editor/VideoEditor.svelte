<script>
  import { onMount, tick } from 'svelte';
  import { invoke, convertFileSrc } from '@tauri-apps/api/core';
  import { listen } from '@tauri-apps/api/event';
  import { getCurrentWindow } from '@tauri-apps/api/window';
  import Timeline from './Timeline.svelte';
  import ExportPanel from './ExportPanel.svelte';
  import BackgroundPicker from './BackgroundPicker.svelte';

  // ─── State ───
  let videoPath = $state('');
  let metadata = $state(null);
  let thumbnails = $state([]);
  let waveform = $state(null);
  let loading = $state(true);
  let loadingMessage = $state('Loading video...');

  // Playback
  let videoEl = $state(null);
  let currentTime = $state(0);
  let isPlaying = $state(false);
  let duration = $state(0);

  // Trim
  let trimStart = $state(0);
  let trimEnd = $state(0);

  // Cuts (split points within trim range)
  let cutPoints = $state([]);
  let deletedSegments = $state(new Set());

  // Derived: segments from cut points
  let segments = $derived.by(() => {
    const pts = [trimStart, ...cutPoints.filter(t => t > trimStart + 0.05 && t < trimEnd - 0.05).sort((a, b) => a - b), trimEnd];
    return pts.slice(0, -1).map((start, i) => ({
      index: i,
      start,
      end: pts[i + 1],
      deleted: deletedSegments.has(i),
    }));
  });

  let keptSegments = $derived(segments.filter(s => !s.deleted));

  // Export
  let showExportPanel = $state(false);
  let outputFormat = $state('mp4');

  // Background / style
  let background = $state({ type: 'transparent' });
  let padding = $state(0);
  let cornerRadius = $state(0);
  let shadowEnabled = $state(false);
  let shadowIntensity = $state(73);

  // Zoom
  let zoomKeyframes = $state([]);
  let zoomEnabled = $state(true);
  let tilt3dEnabled = $state(false); // global 3D tilt toggle
  const MAX_TILT = 16; // degrees
  const PERSPECTIVE = 650; // px — closer = more dramatic

  // Speed
  let speedSegments = $state([]);
  let speedIdCounter = $state(0);

  // Selected segment from timeline (for sidebar editing)
  let selectedSeg = $state(null); // { type: 'zoom'|'speed', ... }
  let selectedCutSegment = $state(null); // index of selected cut segment

  // Presets
  const zoomPresets = [1.25, 1.5, 1.8, 2.2, 3.5, 5.0];
  const speedPresets = [0.25, 0.5, 0.75, 1.25, 1.5, 1.75, 2.0];

  // Computed
  let trimDuration = $derived(trimEnd - trimStart);
  let filename = $derived(videoPath.split('/').pop()?.replace(/\.[^.]+$/, '') || 'Untitled');
  let hasBackground = $derived(background.type !== 'transparent');
  // When zooming, smoothly transition to full-canvas mode (no bg/padding/radius)
  let isZoomed = $derived(currentZoom.zoom > 1.01);
  let zoomBlendFactor = $derived.by(() => {
    const z = currentZoom.zoom;
    if (z <= 1.01) return 0;
    // Quick ramp: fully blended by 1.3x zoom
    return Math.min(1, (z - 1.0) / 0.3);
  });

  // ─── Zoom Interpolation (FocuSee-style "breathe" model) ───

  const TRANSITION_IN = 0.55;   // slower zoom-in (was 0.30 — too snappy)
  const TRANSITION_OUT = 0.60;  // slower zoom-out (was 0.35)
  const DEFAULT_HOLD = 0.6;     // slightly longer hold at peak
  const MERGE_GAP = 1.0;        // wider gap before treating as separate segment

  function springEaseIn(t) {
    const c = 1.2;
    const p = t - 1;
    return p * p * ((c + 1) * p + c) + 1;
  }

  function easeOutCubic(t) {
    return 1 - Math.pow(1 - t, 3);
  }

  function smoothStep(t) {
    return t * t * (3 - 2 * t);
  }

  function interpolateZoom(time, kfs) {
    if (!kfs || kfs.length === 0 || !zoomEnabled) {
      return { zoom: 1.0, centerX: 0.5, centerY: 0.5 };
    }

    const sorted = [...kfs].sort((a, b) => a.time - b.time);

    const segments = [];
    let seg = { kfs: [sorted[0]] };
    for (let i = 1; i < sorted.length; i++) {
      const prevKf = seg.kfs[seg.kfs.length - 1];
      const prevEnd = prevKf.time + (prevKf.hold || DEFAULT_HOLD);
      if (sorted[i].time - prevEnd < MERGE_GAP) {
        seg.kfs.push(sorted[i]);
      } else {
        segments.push(seg);
        seg = { kfs: [sorted[i]] };
      }
    }
    segments.push(seg);

    for (const s of segments) {
      const first = s.kfs[0];
      const last = s.kfs[s.kfs.length - 1];
      s.enterStart = first.time - TRANSITION_IN;
      s.peakStart = first.time;
      s.peakEnd = last.time + (last.hold || DEFAULT_HOLD);
      s.exitEnd = s.peakEnd + TRANSITION_OUT;
    }

    const REST = { zoom: 1.0, centerX: 0.5, centerY: 0.5 };

    for (const s of segments) {
      if (time < s.enterStart || time > s.exitEnd) continue;

      const firstKf = s.kfs[0];
      const lastKf = s.kfs[s.kfs.length - 1];

      if (time < s.peakStart) {
        const t = Math.max(0, Math.min(1, (time - s.enterStart) / TRANSITION_IN));
        const e = springEaseIn(t);
        return {
          zoom: 1.0 + (firstKf.zoom - 1.0) * e,
          centerX: 0.5 + (firstKf.center_x - 0.5) * e,
          centerY: 0.5 + (firstKf.center_y - 0.5) * e,
        };
      }

      if (time > s.peakEnd) {
        const t = Math.max(0, Math.min(1, (time - s.peakEnd) / TRANSITION_OUT));
        const e = 1 - easeOutCubic(t);
        return {
          zoom: 1.0 + (lastKf.zoom - 1.0) * e,
          centerX: 0.5 + (lastKf.center_x - 0.5) * e,
          centerY: 0.5 + (lastKf.center_y - 0.5) * e,
        };
      }

      if (s.kfs.length === 1) {
        return { zoom: firstKf.zoom, centerX: firstKf.center_x, centerY: firstKf.center_y };
      }

      for (let i = 0; i < s.kfs.length; i++) {
        const kf = s.kfs[i];
        const kfEnd = kf.time + (kf.hold || DEFAULT_HOLD);
        if (time >= kf.time && time <= kfEnd) {
          return { zoom: kf.zoom, centerX: kf.center_x, centerY: kf.center_y };
        }
        if (i < s.kfs.length - 1) {
          const next = s.kfs[i + 1];
          if (time > kfEnd && time < next.time) {
            const span = next.time - kfEnd;
            const t = span > 0 ? (time - kfEnd) / span : 0;
            const e = smoothStep(t);
            return {
              zoom: kf.zoom + (next.zoom - kf.zoom) * e,
              centerX: kf.center_x + (next.center_x - kf.center_x) * e,
              centerY: kf.center_y + (next.center_y - kf.center_y) * e,
            };
          }
        }
      }

      return { zoom: lastKf.zoom, centerX: lastKf.center_x, centerY: lastKf.center_y };
    }

    return REST;
  }

  let currentZoom = $derived(interpolateZoom(currentTime, zoomKeyframes));

  // 2D zoom on the <video> element
  // When zoomed: use object-fit:cover + object-position to fill entire container
  // When not zoomed: no transform needed
  let zoomTransformCss = $derived.by(() => {
    const z = currentZoom;
    if (z.zoom <= 1.01) return '';

    const cx = Math.max(0, Math.min(1, z.centerX));
    const cy = Math.max(0, Math.min(1, z.centerY));
    const ox = (cx * 100).toFixed(1);
    const oy = (cy * 100).toFixed(1);

    return `transform: scale(${z.zoom.toFixed(3)}); transform-origin: ${ox}% ${oy}%;`;
  });

  // 3D tilt on the outer card (video-clip container) — FocuSee style
  // The card tilts as a floating screen in 3D space while video zooms inside
  let tilt3dCss = $derived.by(() => {
    if (!tilt3dEnabled) return '';
    const z = currentZoom;
    if (z.zoom <= 1.01) return '';

    const cx = Math.max(0, Math.min(1, z.centerX));
    const cy = Math.max(0, Math.min(1, z.centerY));

    // Ramp: kicks in fast, full at 2x zoom
    const t = Math.min(1, (z.zoom - 1) / 0.8);
    const ease = t * t * (3 - 2 * t); // smoothstep

    const dx = cx - 0.5;
    const dy = cy - 0.5;
    const rotateY = dx * -MAX_TILT * ease;
    const rotateX = dy * MAX_TILT * 0.85 * ease;

    // Dynamic shadow that shifts opposite to tilt direction (floating card feel)
    const shadowX = (dx * 20 * ease).toFixed(1);
    const shadowY = (8 + dy * 15 * ease).toFixed(1);
    const shadowBlur = (20 + ease * 30).toFixed(0);
    const shadowOpacity = (0.25 + ease * 0.2).toFixed(2);

    return `transform: perspective(${PERSPECTIVE}px) rotateX(${rotateX.toFixed(2)}deg) rotateY(${rotateY.toFixed(2)}deg); box-shadow: ${shadowX}px ${shadowY}px ${shadowBlur}px rgba(0,0,0,${shadowOpacity});`;
  });

  let previewBgStyle = $derived.by(() => {
    if (background.type === 'gradient') {
      const colors = background.colors || ['#6c5ce7', '#a29bfe'];
      const angle = background.angle || 135;
      return `background: linear-gradient(${angle}deg, ${colors[0]}, ${colors[1]})`;
    }
    if (background.type === 'solid') {
      return `background: ${background.color || '#2d3436'}`;
    }
    if (background.type === 'image') {
      return background.css || '';
    }
    return '';
  });

  let previewVideoCss = $derived.by(() => {
    let css = '';
    const blend = zoomBlendFactor;
    const effectiveRadius = cornerRadius * (1 - blend);
    if (effectiveRadius > 0.5) css += `border-radius: ${effectiveRadius.toFixed(1)}px; `;
    if (shadowEnabled && hasBackground && blend < 0.95) {
      const si = shadowIntensity / 100;
      const shadowAlpha = 1 - blend;
      css += `box-shadow: 0 ${Math.round(si * 12)}px ${Math.round(si * 48)}px rgba(0,0,0,${(si * 0.55 * shadowAlpha).toFixed(2)}), 0 2px 12px rgba(0,0,0,${(0.3 * shadowAlpha).toFixed(2)}); `;
    }
    return css;
  });

  onMount(async () => {
    const params = new URLSearchParams(window.location.search);
    const pathParam = params.get('path');
    if (pathParam) {
      videoPath = decodeURIComponent(pathParam);
    }

    const unlisten = await listen('editor-open', (event) => {
      if (event.payload?.path) {
        videoPath = event.payload.path;
        loadVideo();
      }
    });

    function handleKeydown(e) {
      if (e.target.tagName === 'INPUT' || e.target.tagName === 'TEXTAREA') return;
      switch (e.code) {
        case 'Space':
          e.preventDefault();
          togglePlay();
          break;
        case 'ArrowLeft':
          e.preventDefault();
          seekTo(Math.max(trimStart, currentTime - (e.shiftKey ? 5 : 1)));
          break;
        case 'ArrowRight':
          e.preventDefault();
          seekTo(Math.min(trimEnd, currentTime + (e.shiftKey ? 5 : 1)));
          break;
        case 'KeyE':
          if (e.metaKey || e.ctrlKey) {
            e.preventDefault();
            showExportPanel = true;
          }
          break;
        case 'KeyB':
        case 'KeyS':
          if (!e.metaKey && !e.ctrlKey) {
            e.preventDefault();
            cutAtPlayhead();
          }
          break;
        case 'Delete':
        case 'Backspace':
          if (selectedSeg?.type === 'zoom') {
            e.preventDefault();
            deleteSelectedZoom();
          } else if (selectedSeg?.type === 'speed') {
            e.preventDefault();
            deleteSelectedSpeed();
          } else if (selectedCutSegment !== null) {
            e.preventDefault();
            toggleDeleteSegment(selectedCutSegment);
            selectedCutSegment = null;
          }
          break;
        case 'Escape':
          showExportPanel = false;
          selectedSeg = null;
          selectedCutSegment = null;
          break;
      }
    }
    window.addEventListener('keydown', handleKeydown);

    if (videoPath) {
      await loadVideo();
    }

    return () => {
      unlisten();
      window.removeEventListener('keydown', handleKeydown);
    };
  });

  async function loadVideo() {
    loading = true;
    try {
      loadingMessage = 'Loading...';
      metadata = await invoke('get_video_metadata', { path: videoPath });
      duration = metadata.duration_secs;
      trimEnd = duration;

      loading = false;
      await tick();

      if (videoEl) {
        videoEl.src = convertFileSrc(videoPath);
        await new Promise((resolve) => {
          videoEl.onloadeddata = resolve;
          videoEl.onerror = () => resolve();
        });
      }

      generateThumbnailsAsync();
      generateWaveformAsync();
      autoApplyZoom();
    } catch (e) {
      console.error('Failed to load video:', e);
      loading = true;
      loadingMessage = `Error: ${e}`;
    }
  }

  async function generateThumbnailsAsync() {
    try {
      thumbnails = await invoke('generate_timeline_thumbnails', {
        path: videoPath, count: 30, thumbHeight: 54,
      });
    } catch (e) { console.error('Thumbnail generation failed:', e); }
  }

  async function generateWaveformAsync() {
    try {
      waveform = await invoke('generate_waveform', {
        path: videoPath, numSamples: 200,
      });
    } catch (e) { console.error('Waveform generation failed:', e); }
  }

  async function autoApplyZoom() {
    try {
      const suggested = await invoke('suggest_zoom_keyframes', { videoPath });
      if (suggested.length === 0) return;

      // Keyframes already have normalized 0-1 coordinates from Rust
      zoomKeyframes = suggested.map(kf => ({
        ...kf,
        center_x: Math.max(0, Math.min(1, kf.center_x)),
        center_y: Math.max(0, Math.min(1, kf.center_y)),
        zoom: Math.min(3.0, Math.max(1.2, kf.zoom)),
      }));
    } catch (e) {
      console.log('[editor] No mouse track:', e);
    }
  }

  // ─── Playback ───

  function togglePlay() {
    if (!videoEl) return;
    if (isPlaying) {
      videoEl.pause();
    } else {
      if (videoEl.currentTime >= trimEnd - 0.05) {
        videoEl.currentTime = trimStart;
      }
      videoEl.play();
    }
    isPlaying = !isPlaying;
  }

  function handleTimeUpdate() {
    if (!videoEl) return;
    currentTime = videoEl.currentTime;
    if (currentTime >= trimEnd) {
      videoEl.pause();
      isPlaying = false;
      videoEl.currentTime = trimEnd;
      currentTime = trimEnd;
      return;
    }
    // Skip over deleted segments during playback
    if (isPlaying && segments.length > 1) {
      const inDeleted = segments.find(s => s.deleted && currentTime >= s.start && currentTime < s.end);
      if (inDeleted) {
        // Jump to next kept segment
        const nextKept = segments.find(s => !s.deleted && s.start >= inDeleted.end);
        if (nextKept) {
          videoEl.currentTime = nextKept.start;
          currentTime = nextKept.start;
        } else {
          videoEl.pause();
          isPlaying = false;
        }
      }
    }
  }

  function seekTo(time) {
    if (!videoEl) return;
    videoEl.currentTime = time;
    currentTime = time;
  }

  function onTrimChange(start, end) {
    trimStart = start;
    trimEnd = end;
    // Remove cut points outside new trim range
    cutPoints = cutPoints.filter(t => t > start + 0.05 && t < end - 0.05);
    if (currentTime < start || currentTime > end) seekTo(start);
  }

  // ─── Cuts ───

  function cutAtPlayhead() {
    const t = currentTime;
    if (t <= trimStart + 0.15 || t >= trimEnd - 0.15) return;
    if (cutPoints.some(c => Math.abs(c - t) < 0.2)) return; // too close to existing
    cutPoints = [...cutPoints, t].sort((a, b) => a - b);
  }

  function toggleDeleteSegment(index) {
    const next = new Set(deletedSegments);
    if (next.has(index)) next.delete(index);
    else next.add(index);
    // Don't allow deleting ALL segments
    const remaining = segments.filter(s => !next.has(s.index));
    if (remaining.length === 0) return;
    deletedSegments = next;
  }

  function removeCutPoint(time) {
    const idx = cutPoints.findIndex(c => Math.abs(c - time) < 0.1);
    if (idx < 0) return;
    // Merge the two segments around this cut point
    // If either was deleted, keep the merged one as not-deleted
    const segIdx = cutPoints.slice(0, idx + 1).length; // segment AFTER this cut
    const next = new Set(deletedSegments);
    next.delete(segIdx);
    // Renumber: shift down all indices > segIdx
    const renumbered = new Set();
    for (const i of next) {
      if (i < segIdx) renumbered.add(i);
      else if (i > segIdx) renumbered.add(i - 1);
    }
    deletedSegments = renumbered;
    cutPoints = cutPoints.filter((_, i) => i !== idx);
  }

  function clearAllCuts() {
    cutPoints = [];
    deletedSegments = new Set();
  }

  function onBackgroundChange(bg) {
    background = bg;
    if (bg.type !== 'transparent' && padding === 0) {
      padding = 48;
      cornerRadius = 12;
      shadowEnabled = true;
    }
  }

  // ─── Zoom presets ───
  function addZoomAtPlayhead(zoomLevel) {
    const newKf = {
      time: currentTime,
      zoom: zoomLevel,
      center_x: 0.5,
      center_y: 0.5,
      easing: 'spring',
      hold: 0.5,
    };
    const threshold = 0.3;
    const nearIdx = zoomKeyframes.findIndex(kf => Math.abs(kf.time - currentTime) < threshold);
    if (nearIdx >= 0) {
      const updated = [...zoomKeyframes];
      updated[nearIdx] = { ...updated[nearIdx], zoom: zoomLevel };
      zoomKeyframes = updated;
    } else {
      zoomKeyframes = [...zoomKeyframes, newKf].sort((a, b) => a.time - b.time);
    }
    zoomEnabled = true;
  }

  // Add zoom at a specific time (from timeline + button)
  function addZoomAtTime(time, zoomLevel) {
    const newKf = {
      time,
      zoom: zoomLevel,
      center_x: 0.5,
      center_y: 0.5,
      easing: 'spring',
      hold: 0.5,
    };
    const threshold = 0.3;
    const nearIdx = zoomKeyframes.findIndex(kf => Math.abs(kf.time - time) < threshold);
    if (nearIdx >= 0) {
      const updated = [...zoomKeyframes];
      updated[nearIdx] = { ...updated[nearIdx], zoom: zoomLevel };
      zoomKeyframes = updated;
    } else {
      zoomKeyframes = [...zoomKeyframes, newKf].sort((a, b) => a.time - b.time);
    }
    zoomEnabled = true;
  }

  // Add speed at a specific time (from timeline + button)
  function addSpeedAtTime(time, speed) {
    const halfDur = 1.5;
    const start = Math.max(trimStart, time - halfDur);
    const end = Math.min(trimEnd, time + halfDur);
    const existing = speedSegments.findIndex(s => time >= s.start && time <= s.end);
    if (existing >= 0) {
      const updated = [...speedSegments];
      updated[existing] = { ...updated[existing], speed };
      speedSegments = updated;
    } else {
      speedIdCounter++;
      speedSegments = [...speedSegments, { id: speedIdCounter, start, end, speed }];
    }
  }

  // ─── Speed segments ───
  function addSpeedSegment(speed) {
    const halfDur = 1.5;
    const start = Math.max(trimStart, currentTime - halfDur);
    const end = Math.min(trimEnd, currentTime + halfDur);
    const existing = speedSegments.findIndex(s => currentTime >= s.start && currentTime <= s.end);
    if (existing >= 0) {
      const updated = [...speedSegments];
      updated[existing] = { ...updated[existing], speed };
      speedSegments = updated;
    } else {
      speedIdCounter++;
      speedSegments = [...speedSegments, { id: speedIdCounter, start, end, speed }];
    }
  }

  function removeSpeedSegment(id) {
    speedSegments = speedSegments.filter(s => s.id !== id);
  }

  // ─── Sidebar segment editing (from timeline selection) ───
  function onSelectionChange(sel) {
    selectedSeg = sel; // { type, value, index/id } or null
  }

  function updateSelectedZoomValue(value) {
    if (!selectedSeg || selectedSeg.type !== 'zoom') return;
    selectedSeg = { ...selectedSeg, value };
    // Dispatch to timeline's zoom update logic
    // We need to update keyframes for the segment group
    const TRANSITION_IN_L = 0.55;
    const TRANSITION_OUT_L = 0.60;
    const DEFAULT_HOLD_L = 0.6;
    const MERGE_GAP_L = 1.0;
    const sorted = [...zoomKeyframes].sort((a, b) => a.time - b.time);
    if (sorted.length === 0) return;
    const groups = [];
    let grp = { kfs: [sorted[0]], indices: [0] };
    for (let i = 1; i < sorted.length; i++) {
      const prev = grp.kfs[grp.kfs.length - 1];
      const prevEnd = prev.time + (prev.hold || DEFAULT_HOLD_L);
      if (sorted[i].time - prevEnd < MERGE_GAP_L) {
        grp.kfs.push(sorted[i]);
        grp.indices.push(i);
      } else {
        groups.push(grp);
        grp = { kfs: [sorted[i]], indices: [i] };
      }
    }
    groups.push(grp);
    const seg = groups[selectedSeg.index];
    if (!seg) return;
    const updated = [...zoomKeyframes];
    for (const kfIdx of seg.indices) {
      if (sorted[kfIdx]) {
        const origIdx = zoomKeyframes.findIndex(k => k.time === sorted[kfIdx].time);
        if (origIdx >= 0) updated[origIdx] = { ...updated[origIdx], zoom: value };
      }
    }
    zoomKeyframes = updated;
  }

  function deleteSelectedZoom() {
    if (!selectedSeg || selectedSeg.type !== 'zoom') return;
    const sorted = [...zoomKeyframes].sort((a, b) => a.time - b.time);
    if (sorted.length === 0) return;
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
    const seg = groups[selectedSeg.index];
    if (!seg) return;
    const timesToRemove = new Set(seg.indices.map(i => sorted[i]?.time));
    zoomKeyframes = zoomKeyframes.filter(k => !timesToRemove.has(k.time));
    selectedSeg = null;
  }

  function updateSelectedSpeedValue(value) {
    if (!selectedSeg || selectedSeg.type !== 'speed') return;
    const id = selectedSeg.id;
    selectedSeg = { ...selectedSeg, value };
    speedSegments = speedSegments.map(s =>
      s.id === id ? { ...s, speed: value } : s
    );
  }

  function deleteSelectedSpeed() {
    if (!selectedSeg || selectedSeg.type !== 'speed') return;
    const id = selectedSeg.id;
    speedSegments = speedSegments.filter(s => s.id !== id);
    selectedSeg = null;
  }

  function getSpeedAtTime(time) {
    const seg = speedSegments.find(s => time >= s.start && time <= s.end);
    return seg ? seg.speed : 1.0;
  }

  // Sync playback rate with speed segments
  $effect(() => {
    if (videoEl && isPlaying) {
      const rate = getSpeedAtTime(currentTime);
      if (videoEl.playbackRate !== rate) {
        videoEl.playbackRate = rate;
      }
    }
  });

  async function closeEditor() {
    await getCurrentWindow().destroy();
  }

  function formatTime(secs) {
    if (!secs || secs < 0) return '0:00';
    const m = Math.floor(secs / 60);
    const s = Math.floor(secs % 60);
    const ms = Math.floor((secs % 1) * 10);
    return `${m}:${s.toString().padStart(2, '0')}.${ms}`;
  }

  function formatSize(bytes) {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="editor">
  {#if loading}
    <div class="loading-screen">
      <div class="loading-spinner"></div>
      <div class="loading-text">{loadingMessage}</div>
    </div>
  {:else}
    <!-- ─── Title Bar ─── -->
    <div class="titlebar">
      <div class="titlebar-left">
        <button class="traffic-btn traffic-close" onclick={closeEditor} title="Close">
          <svg width="6" height="6" viewBox="0 0 6 6"><path d="M0.5 0.5L5.5 5.5M5.5 0.5L0.5 5.5" stroke="currentColor" stroke-width="1.2"/></svg>
        </button>
      </div>
      <div class="titlebar-center">
        <span class="titlebar-name">{filename}</span>
        <span class="titlebar-meta">{metadata?.width}x{metadata?.height} &middot; {formatSize(metadata?.file_size_bytes || 0)}</span>
      </div>
      <div class="titlebar-right"></div>
    </div>

    <!-- ─── Main: Preview + Sidebar ─── -->
    <div class="main-area">
      <!-- Preview Section -->
      <div class="preview-section">
        <div class="preview-canvas" class:zoomed={isZoomed}>
          <!-- Background layer: fills the entire canvas independently -->
          {#if hasBackground}
            <div class="preview-bg" style="{previewBgStyle}; opacity: {1 - zoomBlendFactor}"></div>
          {/if}
          <!-- Video card: floats on top of background, inset by padding -->
          <div class="preview-stage" style={hasBackground && !isZoomed ? `padding: ${padding * (1 - zoomBlendFactor)}px` : ''}>
            <div class="video-clip" class:zoomed={isZoomed} style="{previewVideoCss}{tilt3dCss}">
              <!-- svelte-ignore a11y_media_has_caption -->
              <video
                bind:this={videoEl}
                class="video"
                class:zoomed={isZoomed}
                style={zoomTransformCss}
                ontimeupdate={handleTimeUpdate}
                onended={() => isPlaying = false}
                preload="auto"
                playsinline
              ></video>
            </div>
          </div>
          <!-- Play button overlay -->
          <button class="play-overlay" onclick={togglePlay} class:visible={!isPlaying}>
            {#if isPlaying}
              <svg width="48" height="48" viewBox="0 0 24 24" fill="white"><rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/></svg>
            {:else}
              <svg width="48" height="48" viewBox="0 0 24 24" fill="white"><path d="M8 5v14l11-7z"/></svg>
            {/if}
          </button>
        </div>

        <!-- Playback Controls -->
        <div class="playback-bar">
          <button class="pb-play" onclick={togglePlay}>
            {#if isPlaying}
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><rect x="6" y="4" width="4" height="16" rx="1"/><rect x="14" y="4" width="4" height="16" rx="1"/></svg>
            {:else}
              <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
            {/if}
          </button>
          <span class="pb-time">{formatTime(currentTime)}<span class="pb-dim"> / {formatTime(duration)}</span></span>
          <div class="pb-spacer"></div>
          <button class="pb-icon" class:active={zoomEnabled} onclick={() => zoomEnabled = !zoomEnabled} title="Auto Zoom">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/><line x1="11" y1="8" x2="11" y2="14"/><line x1="8" y1="11" x2="14" y2="11"/></svg>
            {#if zoomKeyframes.length > 0}<span class="pb-badge">{zoomKeyframes.length}</span>{/if}
          </button>
        </div>
      </div>

      <!-- ─── Sidebar ─── -->
      <div class="sidebar">
        <div class="sidebar-scroll">
          <!-- Background: compact dot row -->
          <section class="sb-section">
            <div class="sb-header"><span>Background</span></div>
            <BackgroundPicker {background} {onBackgroundChange} />
          </section>

          <!-- Style sliders: always visible, compact -->
          <section class="sb-section">
            <div class="sb-header"><span>Style</span></div>
            <div class="fx-row">
              <span class="fx-label">Shadow</span>
              <input type="range" min="0" max="100" step="5" value={shadowIntensity} oninput={(e) => { shadowIntensity = Number(e.target.value); shadowEnabled = shadowIntensity > 0; }} />
              <span class="fx-val">{shadowEnabled ? `${shadowIntensity}%` : 'Off'}</span>
            </div>
            <div class="fx-row">
              <span class="fx-label">Round</span>
              <input type="range" min="0" max="40" step="1" value={cornerRadius} oninput={(e) => cornerRadius = Number(e.target.value)} />
              <span class="fx-val">{cornerRadius}px</span>
            </div>
            <div class="fx-row">
              <span class="fx-label">Padding</span>
              <input type="range" min="0" max="120" step="4" value={padding} oninput={(e) => padding = Number(e.target.value)} />
              <span class="fx-val">{Math.round(padding / 1.2)}%</span>
            </div>
          </section>

          <!-- Zoom section -->
          <section class="sb-section">
            <div class="sb-header">
              <span>Zoom</span>
              {#if zoomKeyframes.length > 0}<span class="sb-count">{zoomKeyframes.length}</span>{/if}
            </div>
            <!-- Selected zoom editor (contextual) -->
            {#if selectedSeg?.type === 'zoom'}
              <div class="seg-editor">
                <div class="seg-editor-row">
                  <span class="seg-editor-val">{selectedSeg.value.toFixed(1)}x</span>
                  <button class="seg-editor-del" onclick={deleteSelectedZoom} title="Delete">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
                  </button>
                </div>
                <input type="range" class="seg-slider zoom-slider" min="1.1" max="5" step="0.1" value={selectedSeg.value} oninput={(e) => updateSelectedZoomValue(parseFloat(e.target.value))} />
              </div>
            {/if}
            <div class="preset-row">
              {#each zoomPresets as z}
                <button class="chip chip-zoom" onclick={() => addZoomAtPlayhead(z)}>{z}x</button>
              {/each}
            </div>
            <div class="action-row">
              <button class="action-sm" onclick={autoApplyZoom}>Auto</button>
              <button class="action-sm" class:active={tilt3dEnabled} onclick={() => tilt3dEnabled = !tilt3dEnabled}>3D</button>
              {#if zoomKeyframes.length > 0}
                <button class="action-sm danger" onclick={() => { zoomKeyframes = []; selectedSeg = null; }}>Clear</button>
              {/if}
            </div>
          </section>

          <!-- Speed section -->
          <section class="sb-section">
            <div class="sb-header">
              <span>Speed</span>
              {#if speedSegments.length > 0}<span class="sb-count speed">{speedSegments.length}</span>{/if}
            </div>
            {#if selectedSeg?.type === 'speed'}
              <div class="seg-editor">
                <div class="seg-editor-row">
                  <span class="seg-editor-val">{selectedSeg.value.toFixed(2)}x</span>
                  <button class="seg-editor-del" onclick={deleteSelectedSpeed} title="Delete">
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
                  </button>
                </div>
                <input type="range" class="seg-slider speed-slider" min="0.1" max="3" step="0.05" value={selectedSeg.value} oninput={(e) => updateSelectedSpeedValue(parseFloat(e.target.value))} />
              </div>
            {/if}
            <div class="preset-row">
              {#each speedPresets as s}
                <button class="chip chip-speed" onclick={() => addSpeedSegment(s)}>{s}x</button>
              {/each}
            </div>
            {#if speedSegments.length > 0}
              <div class="action-row">
                <button class="action-sm danger" onclick={() => { speedSegments = []; selectedSeg = null; }}>Clear</button>
              </div>
            {/if}
          </section>
        </div>

        <!-- Sidebar Footer: Export -->
        <div class="sb-footer">
          <div class="format-row">
            <button class="fmt-btn" class:active={outputFormat === 'mp4'} onclick={() => outputFormat = 'mp4'}>MP4</button>
            <button class="fmt-btn" class:active={outputFormat === 'gif'} onclick={() => outputFormat = 'gif'}>GIF</button>
          </div>
          <button class="export-btn" onclick={() => showExportPanel = true}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>
            Export
          </button>
        </div>
      </div>
    </div>

    <!-- ─── Bottom Panel: Timeline Toolbar + Timeline ─── -->
    <div class="bottom-panel">
      <div class="timeline-toolbar">
        <div class="tt-left">
          <button class="tt-btn tt-split" onclick={cutAtPlayhead} title="Split at playhead (B)">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><line x1="20" y1="4" x2="8.12" y2="15.88"/><line x1="14.47" y1="14.48" x2="20" y2="20"/><line x1="8.12" y1="8.12" x2="12" y2="12"/></svg>
            <span>Split</span>
            <kbd>B</kbd>
          </button>
          {#if cutPoints.length > 0}
            <button class="tt-btn tt-danger" onclick={clearAllCuts} title="Remove all cuts">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6"/></svg>
              <span>Clear Cuts</span>
            </button>
          {/if}
          {#if selectedCutSegment !== null}
            <button class="tt-btn tt-danger" onclick={() => { toggleDeleteSegment(selectedCutSegment); }} title="Delete selected segment (⌫)">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
              <span>Delete Segment</span>
              <kbd>⌫</kbd>
            </button>
          {/if}
        </div>
        <div class="tt-center">
          {#if segments.length > 1}
            <span class="tt-info">Keeping {keptSegments.length}/{segments.length} · {formatTime(keptSegments.reduce((s, seg) => s + seg.end - seg.start, 0))}</span>
          {/if}
        </div>
        <div class="tt-right">
          {#if speedSegments.length > 0}
            <span class="tt-badge speed-badge">
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>
              {speedSegments.length}
            </span>
          {/if}
          {#if zoomKeyframes.length > 0}
            <span class="tt-badge zoom-badge">
              <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5"><circle cx="11" cy="11" r="8"/><line x1="21" y1="21" x2="16.65" y2="16.65"/></svg>
              {zoomKeyframes.length}
            </span>
          {/if}
        </div>
      </div>
      <Timeline
        {thumbnails}
        {waveform}
        {duration}
        {currentTime}
        {trimStart}
        {trimEnd}
        {zoomKeyframes}
        {zoomEnabled}
        {speedSegments}
        {cutPoints}
        {deletedSegments}
        selectedClipIndex={selectedCutSegment}
        onSeek={seekTo}
        {onTrimChange}
        onZoomKeyframeChange={(kfs) => zoomKeyframes = kfs}
        onSpeedSegmentChange={(segs) => speedSegments = segs}
        {onSelectionChange}
        onAddZoomAtTime={addZoomAtTime}
        onAddSpeedAtTime={addSpeedAtTime}
        onCutAtTime={cutAtPlayhead}
        onClipSelect={(idx) => selectedCutSegment = idx}
        onClipDelete={toggleDeleteSegment}
        onJoinClips={removeCutPoint}
      />
    </div>

    <!-- Export Modal -->
    {#if showExportPanel}
      <ExportPanel
        videoPath={videoPath}
        {trimStart}
        {trimEnd}
        {duration}
        {background}
        {padding}
        {cornerRadius}
        {shadowEnabled}
        {zoomKeyframes}
        {zoomEnabled}
        {keptSegments}
        cursorEnabled={false}
        cursorStyle="pointer"
        cursorSize={24}
        cursorColor="#000000"
        showCursorHighlight={false}
        showClickRipple={false}
        onClose={() => showExportPanel = false}
      />
    {/if}
  {/if}
</div>

<style>
  .editor {
    width: 100vw;
    height: 100vh;
    display: flex;
    flex-direction: column;
    background: #0c0c0e;
    color: #e8e8ec;
    overflow: hidden;
    font-family: -apple-system, BlinkMacSystemFont, 'SF Pro Text', 'Segoe UI', sans-serif;
  }

  /* ─── Loading ─── */
  .loading-screen {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 16px;
  }

  .loading-spinner {
    width: 28px;
    height: 28px;
    border: 2px solid rgba(255,255,255,0.08);
    border-top-color: #6c5ce7;
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }

  @keyframes spin { to { transform: rotate(360deg); } }

  .loading-text { font-size: 13px; color: #666; }

  /* ─── Title Bar ─── */
  .titlebar {
    display: flex;
    align-items: center;
    height: 38px;
    padding: 0 12px;
    background: #0c0c0e;
    -webkit-app-region: drag;
    flex-shrink: 0;
  }

  .titlebar-left, .titlebar-right {
    width: 60px;
    flex-shrink: 0;
    -webkit-app-region: no-drag;
  }

  .titlebar-center {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    pointer-events: none;
  }

  .titlebar-name {
    font-size: 12px;
    font-weight: 600;
    color: #999;
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .titlebar-meta { font-size: 10px; color: #555; letter-spacing: 0.2px; }

  .traffic-btn {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    border: none;
    display: flex;
    align-items: center;
    justify-content: center;
    cursor: pointer;
    transition: all 0.15s;
    padding: 0;
  }

  .traffic-close { background: #ff5f57; color: transparent; }
  .traffic-close:hover { color: rgba(0,0,0,0.6); }

  /* ─── Main Area (Preview + Sidebar) ─── */
  .main-area {
    flex: 1;
    display: flex;
    min-height: 0;
  }

  /* ─── Preview Section ─── */
  .preview-section {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .preview-canvas {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    min-height: 0;
    border-radius: 10px;
    margin: 0 16px;
    overflow: hidden;
    position: relative;
  }

  /* Background: absolute fill layer, separate from video */
  .preview-bg {
    position: absolute;
    inset: 0;
    border-radius: 10px;
    transition: background 0.3s;
  }

  /* Stage: positions the video card, inset by padding */
  .preview-stage {
    position: relative;
    z-index: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 100%;
    height: 100%;
    transition: padding 0.15s cubic-bezier(0.22, 0.68, 0.35, 1.0);
    box-sizing: border-box;
  }

  .video-clip {
    position: relative;
    overflow: hidden;
    max-width: 100%;
    max-height: calc(100vh - 260px);
    line-height: 0;
    background: #000;
    /* FocuSee-style: outer card does 3D tilt (floating screen feel) */
    transition: transform 0.4s cubic-bezier(0.22, 0.68, 0.35, 1.0),
                box-shadow 0.4s ease,
                border-radius 0.2s ease;
    will-change: transform;
    transform-style: preserve-3d;
    -webkit-transform-style: preserve-3d;
  }

  /* When zoomed: clip fills entire stage */
  .video-clip.zoomed {
    width: 100%;
    height: 100%;
    max-height: 100%;
    border-radius: 0 !important;
    box-shadow: none !important;
  }

  .video {
    display: block;
    width: 100%;
    height: 100%;
    max-width: 100%;
    max-height: calc(100vh - 260px);
    object-fit: contain;
    /* Inner video: smooth 2D zoom only */
    transition: transform 0.25s cubic-bezier(0.22, 0.68, 0.35, 1.0);
    will-change: transform;
    backface-visibility: hidden;
    -webkit-backface-visibility: hidden;
  }

  /* When zoomed: video covers full area, zoom/pan via transform */
  .video.zoomed {
    object-fit: cover;
    max-height: 100%;
  }

  .play-overlay {
    position: absolute;
    inset: 0;
    z-index: 2;
    display: flex;
    align-items: center;
    justify-content: center;
    background: transparent;
    border: none;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.2s;
  }

  .play-overlay:hover, .play-overlay.visible:hover {
    opacity: 1;
    background: rgba(0,0,0,0.25);
  }

  /* ─── Playback Bar ─── */
  .playback-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 16px;
    flex-shrink: 0;
  }

  .pb-play {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: none;
    border-radius: 50%;
    background: rgba(255,255,255,0.08);
    color: #ddd;
    cursor: pointer;
    transition: all 0.15s;
  }

  .pb-play:hover { background: rgba(255,255,255,0.15); color: #fff; }

  .pb-time {
    font-size: 12px;
    font-weight: 600;
    color: #ccc;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.2px;
  }

  .pb-dim { color: #555; }
  .pb-spacer { flex: 1; }

  .pb-icon {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 5px 8px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: #777;
    cursor: pointer;
    transition: all 0.15s;
  }

  .pb-icon:hover { background: rgba(255,255,255,0.08); color: #ddd; }
  .pb-icon.active { background: rgba(108,92,231,0.15); color: #a29bfe; }

  .pb-badge {
    font-size: 9px;
    font-weight: 700;
    min-width: 14px;
    height: 14px;
    line-height: 14px;
    text-align: center;
    border-radius: 7px;
    background: rgba(108,92,231,0.25);
    color: #a29bfe;
  }

  /* ─── Sidebar ─── */
  .sidebar {
    width: 280px;
    flex-shrink: 0;
    display: flex;
    flex-direction: column;
    background: #111113;
    border-left: 1px solid rgba(255,255,255,0.04);
  }

  .sidebar-scroll {
    flex: 1;
    overflow-y: auto;
    overflow-x: hidden;
    padding: 4px 0;
  }

  .sidebar-scroll::-webkit-scrollbar { width: 4px; }
  .sidebar-scroll::-webkit-scrollbar-track { background: transparent; }
  .sidebar-scroll::-webkit-scrollbar-thumb { background: rgba(255,255,255,0.08); border-radius: 2px; }

  /* ─── Sidebar Sections ─── */
  .sb-section {
    padding: 12px 14px;
    border-bottom: 1px solid rgba(255,255,255,0.04);
  }

  .sb-header {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 11px;
    font-weight: 600;
    color: #666;
    margin-bottom: 8px;
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  .sb-count {
    font-size: 9px;
    font-weight: 700;
    padding: 1px 5px;
    border-radius: 8px;
    background: rgba(0,184,148,0.15);
    color: #00b894;
  }

  .sb-count.speed {
    background: rgba(243,156,18,0.15);
    color: #f39c12;
  }

  /* ─── Preset Chips ─── */
  .preset-row {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
    margin-bottom: 6px;
  }

  .chip {
    padding: 4px 8px;
    border: 1px solid rgba(255,255,255,0.06);
    border-radius: 4px;
    background: transparent;
    color: #777;
    font-size: 10px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.12s;
  }

  .chip:hover { background: rgba(255,255,255,0.06); color: #ddd; }
  .chip-zoom:hover { background: rgba(0,184,148,0.1); color: #55efc4; border-color: rgba(0,184,148,0.3); }
  .chip-speed:hover { background: rgba(243,156,18,0.1); color: #f39c12; border-color: rgba(243,156,18,0.3); }

  .action-row {
    display: flex;
    gap: 4px;
  }

  .action-sm {
    padding: 3px 8px;
    border: 1px solid rgba(255,255,255,0.06);
    border-radius: 4px;
    background: transparent;
    color: #666;
    font-size: 10px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.12s;
  }

  .action-sm:hover { background: rgba(255,255,255,0.06); color: #aaa; }
  .action-sm.active { background: rgba(108,92,231,0.12); border-color: rgba(108,92,231,0.3); color: #a29bfe; }
  .action-sm.danger:hover { background: rgba(255,60,60,0.08); border-color: rgba(255,60,60,0.2); color: #ff6b6b; }

  /* ─── Segment Editor (sidebar) ─── */
  .seg-editor {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .seg-editor-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }

  .seg-editor-val {
    font-size: 18px;
    font-weight: 700;
    color: #e8e8ec;
    font-variant-numeric: tabular-nums;
  }

  .seg-editor-del {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: 1px solid rgba(255,60,60,0.2);
    border-radius: 6px;
    background: rgba(255,60,60,0.06);
    color: #ff6b6b;
    cursor: pointer;
    transition: all 0.15s;
  }

  .seg-editor-del:hover {
    background: rgba(255,60,60,0.15);
    border-color: rgba(255,60,60,0.4);
  }

  .seg-slider {
    width: 100%;
    height: 4px;
    -webkit-appearance: none;
    appearance: none;
    background: rgba(255,255,255,0.08);
    border-radius: 2px;
    outline: none;
  }

  .seg-slider::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.5);
  }

  .zoom-slider::-webkit-slider-runnable-track {
    background: linear-gradient(90deg, rgba(0,184,148,0.15), rgba(0,184,148,0.35));
    border-radius: 2px;
    height: 4px;
  }

  .speed-slider::-webkit-slider-runnable-track {
    background: linear-gradient(90deg, rgba(243,156,18,0.15), rgba(243,156,18,0.35));
    border-radius: 2px;
    height: 4px;
  }

  /* ─── Video Effects ─── */
  .fx-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 8px;
  }

  .fx-row:last-child { margin-bottom: 0; }

  .fx-label {
    font-size: 10px;
    font-weight: 600;
    color: #666;
    width: 60px;
    flex-shrink: 0;
  }

  .fx-row input[type="range"] {
    flex: 1;
    height: 4px;
    -webkit-appearance: none;
    appearance: none;
    background: rgba(255,255,255,0.08);
    border-radius: 2px;
    outline: none;
  }

  .fx-row input[type="range"]::-webkit-slider-thumb {
    -webkit-appearance: none;
    appearance: none;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    cursor: pointer;
    box-shadow: 0 1px 4px rgba(0,0,0,0.5);
  }

  .fx-val {
    font-size: 10px;
    font-weight: 600;
    color: #888;
    font-variant-numeric: tabular-nums;
    width: 32px;
    text-align: right;
    flex-shrink: 0;
  }

  /* ─── Sidebar Footer ─── */
  .sb-footer {
    padding: 12px 14px;
    border-top: 1px solid rgba(255,255,255,0.06);
    display: flex;
    flex-direction: column;
    gap: 8px;
    flex-shrink: 0;
  }

  .format-row {
    display: flex;
    gap: 4px;
  }

  .fmt-btn {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 8px;
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 6px;
    background: transparent;
    color: #888;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
  }

  .fmt-btn:hover { border-color: rgba(255,255,255,0.15); color: #fff; }

  .fmt-btn.active {
    background: rgba(108,92,231,0.15);
    border-color: #6c5ce7;
    color: #a29bfe;
  }

  .export-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 10px 16px;
    border: none;
    border-radius: 8px;
    background: #6c5ce7;
    color: #fff;
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
    width: 100%;
  }

  .export-btn:hover { background: #7d6ff0; }

  /* ─── Bottom Panel ─── */
  .bottom-panel {
    flex-shrink: 0;
    background: #111113;
    border-top: 1px solid rgba(255,255,255,0.04);
  }

  /* ─── Timeline Toolbar ─── */
  .timeline-toolbar {
    display: flex;
    align-items: center;
    padding: 6px 16px;
    gap: 8px;
    border-bottom: 1px solid rgba(255,255,255,0.04);
  }

  .tt-left, .tt-right {
    display: flex;
    align-items: center;
    gap: 4px;
  }

  .tt-left { flex-shrink: 0; }
  .tt-center { flex: 1; display: flex; justify-content: center; }
  .tt-right { flex-shrink: 0; }

  .tt-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 4px 10px;
    border: 1px solid rgba(255,255,255,0.08);
    border-radius: 5px;
    background: rgba(255,255,255,0.03);
    color: #999;
    font-size: 11px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
    white-space: nowrap;
  }

  .tt-btn:hover {
    background: rgba(255,255,255,0.08);
    border-color: rgba(255,255,255,0.15);
    color: #fff;
  }

  .tt-split {
    border-color: rgba(231, 76, 60, 0.25);
    color: #e74c3c;
    background: rgba(231, 76, 60, 0.06);
  }

  .tt-split:hover {
    background: rgba(231, 76, 60, 0.15);
    border-color: rgba(231, 76, 60, 0.5);
    color: #ff6b6b;
  }

  .tt-danger {
    border-color: rgba(255, 60, 60, 0.2);
    color: #ff6b6b;
    background: rgba(255, 60, 60, 0.06);
  }

  .tt-danger:hover {
    background: rgba(255, 60, 60, 0.15);
    border-color: rgba(255, 60, 60, 0.4);
  }

  .tt-btn kbd {
    font-size: 9px;
    font-family: inherit;
    padding: 1px 4px;
    background: rgba(255,255,255,0.06);
    border: 1px solid rgba(255,255,255,0.1);
    border-radius: 3px;
    color: inherit;
    opacity: 0.6;
  }

  .tt-info {
    font-size: 10px;
    color: #555;
    font-variant-numeric: tabular-nums;
  }

  .tt-badge {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: 9px;
    font-weight: 700;
    padding: 2px 7px;
    border-radius: 8px;
  }

  .zoom-badge {
    background: rgba(0,184,148,0.12);
    color: #00b894;
  }

  .speed-badge {
    background: rgba(243,156,18,0.12);
    color: #f39c12;
  }
</style>
