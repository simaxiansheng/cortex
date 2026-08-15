<script lang="ts">
  import { app } from "../lib/store.svelte";
  import * as api from "../lib/api";
  import type { MaterialScope } from "../lib/api";
  import Icon from "../components/Icon.svelte";
  import { jobs, type JobKind } from "../lib/jobs.svelte";

  // ── Material type definitions ──────────────────────────────
  const GEN_TYPES = [
    { id: "flashcards", label: "Flashcards",      ico: "cards", desc: "Spaced-repetition deck",    color: "var(--accent)"      },
    { id: "quiz",       label: "Quiz",             ico: "check", desc: "MCQ · short answer · cloze", color: "var(--info)"        },
    { id: "audio",      label: "Audio overview",   ico: "music", desc: "Two-host podcast",           color: "var(--mode-select)" },
    { id: "slideshow",  label: "Slides",           ico: "grid",  desc: "Presentation slide deck",     color: "var(--warn)"        },
    { id: "infographic",label: "Infographic",      ico: "grid",  desc: "One-poster summary",         color: "var(--ok)"          },
    { id: "mindmap",    label: "Mind map",          ico: "link",  desc: "Concept map of the topic",    color: "var(--info)"        },
  ] as const;

  const srcLabel: Record<string, string> = {
    pdf: "PDF", pptx: "PPTX", docx: "DOCX", web: "WEB", yt: "YT", audio: "AUD", image: "IMG",
  };

  // Count-based formats. Counts have a minimum but deliberately no product-imposed
  // maximum: users can request as many quiz questions or flashcards as they need.
  const COUNT_LIMITS: Record<string, { min: number; def: number }> = {
    flashcards: { min: 4, def: 14 },
    quiz: { min: 3, def: 10 },
  };

  // ── Derived from real active subject ─────────────────────────
  const subjectTopics = $derived(app.activeSubject?.topics ?? []);
  const allSources = $derived(
    subjectTopics.flatMap(t =>
      t.sources.map(s => ({ ...s, topicId: t.id, topicName: t.name }))
    )
  );

  // ── State ─────────────────────────────────────────────────────
  let type  = $state<string>("flashcards");
  let sel   = $state<string[]>([]);
  let title = $state("");
  let customPrompt = $state("");
  // `focus` retrieves only passages related to the student's target before the
  // generation model sees them. `tagged` keeps broad coverage but labels each
  // generated quiz question / flashcard by knowledge point.
  let scope = $state<MaterialScope>("all");
  let focusTopics = $state("");
  // Per-type item count (flashcards / quiz). Seeded from the defaults.
  let cardCount = $state(COUNT_LIMITS.flashcards.def);
  let quizCount = $state(COUNT_LIMITS.quiz.def);

  // ── Derived ───────────────────────────────────────────────────
  const selSources = $derived(allSources.filter(s => sel.includes(s.id)));
  const countLimit = $derived(COUNT_LIMITS[type] ?? null);
  const countValue = $derived(type === "flashcards" ? cardCount : type === "quiz" ? quizCount : null);
  const supportsItemTags = $derived(type === "flashcards" || type === "quiz");
  const needsFocus = $derived(scope === "focus");

  const counts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const s of selSources) c[s.topicName] = (c[s.topicName] ?? 0) + 1;
    return c;
  });
  const topicNames = $derived(Object.keys(counts));
  const autoTopic  = $derived(
    topicNames.length === 0
      ? null
      : [...topicNames].sort((a, b) => counts[b] - counts[a])[0]
  );
  const dominantTopicId = $derived.by(() => {
    if (!autoTopic) return app.activeSubject?.topics[0]?.id;
    const t = subjectTopics.find(t => t.name === autoTopic);
    return t?.id ?? app.activeSubject?.topics[0]?.id;
  });
  const multi = $derived(topicNames.length > 1);

  const tm = $derived(GEN_TYPES.find(t => t.id === type)!);
  const suffixFor = (t: string) =>
    ({ flashcards: " — flashcards", quiz: " — quiz", audio: " — deep dive", slideshow: " — slides", infographic: " — infographic", mindmap: " — mind map" } as Record<string, string>)[t] ?? "";
  const suggested = $derived.by(() => {
    const suffix = suffixFor(type);
    if (selSources.length === 1) {
      const base = selSources[0].name.replace(/\.[^.]+$/, "").trim();
      if (base) return base + suffix;
    }
    return autoTopic ? autoTopic + suffix : "";
  });
  const finalTitle = $derived(title.trim() || suggested);
  const ready = $derived(
    sel.length > 0 && !!app.activeSubject && (!needsFocus || focusTopics.trim().length > 0)
  );

  // ── Actions ───────────────────────────────────────────────────
  function toggle(id: string) {
    sel = sel.includes(id) ? sel.filter(y => y !== id) : [...sel, id];
  }

  function toggleTopic(topicId: string) {
    const topic = subjectTopics.find(t => t.id === topicId);
    if (!topic) return;
    const ids = topic.sources.map(s => s.id);
    const allOn = ids.every(i => sel.includes(i));
    sel = allOn ? sel.filter(i => !ids.includes(i)) : Array.from(new Set([...sel, ...ids]));
  }

  function setCount(n: number) {
    if (!countLimit) return;
    const v = Math.max(countLimit.min, Math.round(n) || countLimit.def);
    if (type === "flashcards") cardCount = v;
    else if (type === "quiz") quizCount = v;
  }

  function selectType(next: string) {
    type = next;
    // Item-level tags only have a clear home on cards and quiz questions.
    if (scope === "tagged" && next !== "flashcards" && next !== "quiz") scope = "all";
  }

  function generate() {
    const sub = app.activeSubject;
    if (!sub) {
      app.pushToast({ kind: "error", title: "No active subject", body: "Select a subject first." });
      return;
    }

    const kind = type as JobKind;
    const subjectId = sub.id;
    const topicId = dominantTopicId;
    const matTitle = finalTitle || undefined;
    const sourceIds = [...sel];
    const count = countValue ?? undefined;
    const focus = focusTopics.trim() || undefined;

    jobs.start({
      kind,
      label: finalTitle || tm.label,
      subjectId,
      topicId,
      run: () =>
        api.generateMaterial(
          subjectId,
          kind as "flashcards" | "quiz" | "audio" | "infographic" | "slideshow" | "mindmap",
          topicId,
          matTitle,
          customPrompt.trim() || undefined,
          sourceIds,
          count,
          scope,
          focus,
        ),
    });

    app.setView("subject");
    app.setTab("materials");
  }

  function cancel() {
    app.setView("subject");
    app.setTab("materials");
  }
</script>

<!-- ── Side-by-side launcher (mirrors AddSource: header · two-column · sticky foot,
     no page scroll — only the long source list scrolls inside its panel). ── -->
<div class="genmat2">
  <!-- Header -->
  <div class="gm2-head">
    <button class="btn btn--icon btn--sm btn--ghost" onclick={cancel} title="Back">
      <span style="display:inline-flex;transform:rotate(180deg)"><Icon name="chevron" size={14} /></span>
    </button>
    <div>
      <div class="eyebrow">Generate material</div>
      <h1 class="addpage-title">New study material</h1>
      <div class="mono faint" style="font-size: var(--t-xs)">
        from {app.activeSubject?.name ?? "your subject"} · pick a format and sources
      </div>
    </div>
  </div>

  <div class="gm2-grid">
    <!-- LEFT — format, count, details -->
    <div class="gm2-left">
      <div class="gm2-block">
        <div class="onb-label mono">FORMAT</div>
        <div class="gm2-formats">
          {#each GEN_TYPES as t (t.id)}
            <button class="gm2-format{type === t.id ? ' on' : ''}" onclick={() => selectType(t.id)}>
              <span class="gm2-format-ico" style:color={t.color}><Icon name={t.ico} size={16} /></span>
              <span class="gm2-format-txt">
                <span class="gm2-format-label">{t.label}</span>
                <span class="gm2-format-desc mono">{t.desc}</span>
              </span>
              {#if type === t.id}<Icon name="check" size={13} color="var(--accent)" />{/if}
            </button>
          {/each}
        </div>
      </div>

      {#if countLimit && countValue !== null}
        <div class="gm2-block">
          <div class="onb-label mono">{type === "quiz" ? "QUESTIONS" : "CARDS"}</div>
          <div class="gm2-count">
            <div class="gm2-step">
              <button class="btn btn--icon btn--sm" onclick={() => setCount(countValue - 1)} aria-label="fewer" disabled={countValue <= countLimit.min}>−</button>
              <input
                class="input mono gm2-step-v gm2-step-input"
                type="number"
                min={countLimit.min}
                step="1"
                value={countValue}
                aria-label={type === "quiz" ? "Number of questions" : "Number of cards"}
                onchange={(event) => setCount(Number(event.currentTarget.value))}
              />
              <button class="btn btn--icon btn--sm" onclick={() => setCount(countValue + 1)} aria-label="more">+</button>
            </div>
            <div class="seg gm2-count-presets">
              {#each [Math.round(countLimit.def / 2), countLimit.def, countLimit.def * 2] as p}
                <button class="seg-opt{countValue === p ? ' on' : ''}" onclick={() => setCount(p)}>{p}</button>
              {/each}
            </div>
          </div>
        </div>
      {/if}

      <div class="gm2-block gm2-scope">
        <div class="onb-label mono">GENERATION SCOPE</div>
        <div class="gm2-scope-options">
          <button
            class="gm2-scope-opt{scope === 'all' ? ' on' : ''}"
            aria-pressed={scope === "all"}
            onclick={() => (scope = "all")}
          >
            <span class="gm2-scope-copy">
              <span>All selected content</span>
              <span class="mono">Use everything in the selected sources.</span>
            </span>
            {#if scope === "all"}<Icon name="check" size={13} color="var(--accent)" />{/if}
          </button>
          <button
            class="gm2-scope-opt{scope === 'focus' ? ' on' : ''}"
            aria-pressed={scope === "focus"}
            onclick={() => (scope = "focus")}
          >
            <span class="gm2-scope-copy">
              <span>Focus knowledge points</span>
              <span class="mono">Retrieve only passages related to what you enter.</span>
            </span>
            {#if scope === "focus"}<Icon name="check" size={13} color="var(--accent)" />{/if}
          </button>
          {#if supportsItemTags}
            <button
              class="gm2-scope-opt{scope === 'tagged' ? ' on' : ''}"
              aria-pressed={scope === "tagged"}
              onclick={() => (scope = "tagged")}
            >
              <span class="gm2-scope-copy">
                <span>All content + item tags</span>
                <span class="mono">Generate broadly and label every question or card.</span>
              </span>
              {#if scope === "tagged"}<Icon name="check" size={13} color="var(--accent)" />{/if}
            </button>
          {/if}
        </div>

        {#if scope === "focus"}
          <div class="field gm2-focus-field">
            <!-- svelte-ignore a11y_label_has_associated_control -->
            <label class="onb-label mono">KNOWLEDGE POINTS TO FOCUS ON <span class="gm2-label-hint">required for focus</span></label>
            <textarea
              class="input set-textarea"
              bind:value={focusTopics}
              rows="3"
              placeholder="e.g. How to find new terms: sources, filtering, validation, and workflow."
            ></textarea>
          </div>
          <p class="gm2-scope-note mono">Cortex first retrieves matching passages from the selected sources. Unrelated passages are excluded.</p>
        {:else if scope === "tagged"}
          <p class="gm2-scope-note mono">Each generated question or card gets 1–3 concise topical tags.</p>
        {/if}
      </div>

      <div class="gm2-block">
        <div class="field">
          <!-- svelte-ignore a11y_label_has_associated_control -->
          <label class="onb-label mono">TITLE <span class="gm2-label-hint">auto-suggested</span></label>
          <input class="input" bind:value={title} placeholder={suggested || "Select sources first…"} />
        </div>
        <div class="field">
          <!-- svelte-ignore a11y_label_has_associated_control -->
          <label class="onb-label mono">CUSTOM INSTRUCTIONS <span class="gm2-label-hint">optional</span></label>
          <textarea
            class="input set-textarea"
            bind:value={customPrompt}
            rows="3"
            placeholder={`e.g. “Focus on exam-likely topics”, “Explain like I'm new to ${app.activeSubject?.name ?? "this"}”, “Emphasise dates and order”…`}
          ></textarea>
        </div>
        <div class="gm-autotag">
          <div class="gm-autotag-l">
            <Icon name="lock" size={13} color="var(--fg-faint)" />
            <span class="mono">Auto-filed under topic</span>
          </div>
          {#if autoTopic}
            <div class="gm-autotag-r">
              <span class="topic-tag mono"><Icon name="chevron" size={9} /> {autoTopic}</span>
              {#if multi}<span class="mono faint">spans {topicNames.length} topics</span>{/if}
            </div>
          {:else}
            <span class="mono faint">select sources to assign a topic</span>
          {/if}
        </div>
      </div>
    </div>

    <!-- RIGHT — source selection (scrolls within the panel) -->
    <div class="gm2-right">
      <div class="gm2-right-head">
        <span class="onb-label mono" style="margin:0">SOURCES</span>
        <span class="faint mono">{sel.length} selected</span>
      </div>
      <div class="gm2-panel">
        {#if subjectTopics.length === 0}
          <p class="mono faint" style="font-size: var(--t-sm); padding: 8px;">No sources found for this subject. Add sources first.</p>
        {:else}
          <div class="gm-sources">
            {#each subjectTopics as topic (topic.id)}
              {@const ids    = topic.sources.map(s => s.id)}
              {@const allOn  = ids.length > 0 && ids.every(i => sel.includes(i))}
              {@const someOn = ids.some(i => sel.includes(i))}
              <div class="gm-topic">
                <div class="gm-topic-h">
                  <button class="gm-check{allOn ? ' on' : someOn ? ' some' : ''}" onclick={() => toggleTopic(topic.id)}>
                    {#if allOn}<Icon name="check" size={11} />{:else if someOn}<span class="gm-dash"></span>{/if}
                  </button>
                  <span class="gm-topic-name mono">{topic.name}</span>
                  <span class="faint mono">{topic.sources.length}</span>
                </div>
                <div class="gm-src-list">
                  {#each topic.sources as s (s.id)}
                    {@const on = sel.includes(s.id)}
                    <button class="gm-src{on ? ' on' : ''}" onclick={() => toggle(s.id)}>
                      <span class="gm-check{on ? ' on' : ''}">
                        {#if on}<Icon name="check" size={11} />{/if}
                      </span>
                      <span class="badge badge--{s.kind === 'audio' ? 'audio' : s.kind}">
                        <span class="dot"></span>{srcLabel[s.kind] ?? s.kind.toUpperCase()}
                      </span>
                      <span class="gm-src-name mono">{s.name}</span>
                      <span class="gm-src-meta mono faint">{s.meta ?? ""}</span>
                    </button>
                  {/each}
                </div>
              </div>
            {/each}
          </div>
        {/if}
      </div>
    </div>
  </div>

  <!-- Sticky footer — always visible, no scrolling to reach Generate -->
  <div class="add-foot gm2-foot">
    <button class="btn btn--ghost" onclick={cancel}>Cancel</button>
    <button class="btn btn--primary" disabled={!ready} onclick={generate}>
      <Icon name="bolt" size={13} /> Generate {tm.label.toLowerCase()}
    </button>
  </div>
</div>

<style>
  .genmat2 {
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: clamp(16px, 3vh, 30px) clamp(20px, 4vw, 52px);
    gap: 16px;
    overflow: hidden;
  }
  .gm2-head { display: flex; align-items: center; gap: 12px; flex: 0 0 auto; }

  .gm2-grid {
    flex: 1 1 auto;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(300px, 380px) 1fr;
    gap: clamp(18px, 3vw, 38px);
    align-items: stretch;
  }
  .gm2-left { display: flex; flex-direction: column; gap: 16px; min-height: 0; overflow-y: auto; padding-right: 4px; }
  .gm2-block { display: flex; flex-direction: column; gap: 8px; }

  /* format picker — vertical rows */
  .gm2-formats { display: flex; flex-direction: column; gap: 6px; }
  .gm2-format {
    display: flex; align-items: center; gap: 11px;
    padding: 9px 11px; text-align: left; cursor: pointer;
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--rad-3);
    color: var(--fg-muted); transition: border-color var(--dur-fast), background var(--dur-fast);
  }
  .gm2-format:hover { border-color: var(--border-strong); color: var(--fg-bright); }
  .gm2-format.on { border-color: var(--accent-dim); background: color-mix(in oklab, var(--accent) 8%, var(--surface)); color: var(--fg-bright); }
  .gm2-format-ico { display: grid; place-items: center; width: 20px; flex: none; }
  .gm2-format-txt { display: flex; flex-direction: column; gap: 1px; flex: 1; min-width: 0; }
  .gm2-format-label { font-size: var(--t-sm); font-weight: 600; }
  .gm2-format-desc { font-size: var(--t-2xs); color: var(--fg-faint); }

  /* count stepper — mirrors the ExamView / Settings pomodoro stepper, with the
     7/14/28 quick-picks as a .seg segmented control. */
  .gm2-count { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .gm2-step { display: flex; align-items: center; gap: 8px; }
  .gm2-step-v { min-width: 36px; text-align: center; color: var(--fg-bright); font-variant-numeric: tabular-nums; }
  .gm2-step-input { width: 72px; min-width: 72px; height: 28px; padding: 0 6px; }
  .gm2-count-presets { margin-left: auto; }

  /* Scope makes the difference between a free-form hint and a strict retrieval
     boundary explicit before the user spends a generation request. */
  .gm2-scope { gap: 8px; }
  .gm2-scope-options { display: flex; flex-direction: column; gap: 6px; }
  .gm2-scope-opt {
    display: flex; align-items: center; gap: 10px; width: 100%; text-align: left;
    padding: 8px 10px; cursor: pointer; color: var(--fg-muted);
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--rad-3);
    transition: border-color var(--dur-fast), background var(--dur-fast), color var(--dur-fast);
  }
  .gm2-scope-opt:hover { border-color: var(--border-strong); color: var(--fg-bright); }
  .gm2-scope-opt.on {
    border-color: var(--accent-dim); color: var(--fg-bright);
    background: color-mix(in oklab, var(--accent) 8%, var(--surface));
  }
  .gm2-scope-copy { display: flex; flex: 1; min-width: 0; flex-direction: column; gap: 1px; font-size: var(--t-sm); font-weight: 600; }
  .gm2-scope-copy .mono { font-size: var(--t-2xs); color: var(--fg-faint); font-weight: 400; line-height: 1.35; }
  .gm2-focus-field { margin-top: 2px; }
  .gm2-scope-note { margin: 0; color: var(--fg-faint); font-size: var(--t-2xs); line-height: 1.45; }

  /* label qualifier — quietly subordinate to the uppercase mono eyebrow label */
  .gm2-label-hint { text-transform: none; letter-spacing: normal; color: var(--fg-faint); font-weight: 400; }

  /* right column / source panel */
  .gm2-right { display: flex; flex-direction: column; min-height: 0; gap: 8px; }
  .gm2-right-head { display: flex; align-items: center; justify-content: space-between; flex: 0 0 auto; }
  .gm2-panel {
    flex: 1 1 auto; min-height: 0; overflow: auto;
    border: 1px solid var(--border); border-radius: var(--rad-3);
    background: var(--surface); padding: 12px;
  }

  .gm2-foot { flex: 0 0 auto; margin: 0; }

  @media (max-width: 860px) {
    .genmat2 { overflow: auto; }
    .gm2-grid { grid-template-columns: 1fr; }
    .gm2-left { overflow: visible; }
    .gm2-panel { max-height: 50vh; }
  }
</style>
