#!/usr/bin/env python3
"""Authoring aid for ops/index.html (gate-transaction review workflow).

The generated file IS the app's source of truth once committed; this
script only exists so the 14 authored rows / detail panels / summary
expressions stay perfectly repetitive. Regenerating overwrites manual
edits to ops/index.html — edit the RECORDS table below instead.

The document follows the reference-dashboard reactive idiom exactly
(one vx-state scope, authored rows + per-record detail panels, vx-show
filtering; vx-for is deferred in reactive profile v0).
"""

from dataclasses import dataclass

@dataclass
class Rec:
    suffix: str          # numeric state suffix, e.g. "2401"
    container: str       # "TCLU 4407123"
    plate: str           # "B 9145 TCA"
    direction: str       # "Gate-in" / "Gate-out"
    lane: str            # "Gate A"
    time: str            # "06:12"
    driver: str          # registered driver name (seed)
    notes: str           # seed notes
    status: str          # CLEARED / PENDING / REJECTED
    reason: str          # short context line, "" for none

RECORDS = [
    Rec("2401", "TCLU 4407123", "B 9145 TCA", "Gate-in",  "Gate A", "06:12", "Budi Santoso",   "Priority unload, bay 3",              "CLEARED",  ""),
    Rec("2402", "MRKU 8890145", "B 9210 DFR", "Gate-in",  "Gate B", "06:40", "Siti Rahayu",    "Customs hold released 07:05",         "CLEARED",  ""),
    Rec("2403", "HLXU 5521038", "B 8761 PKB", "Gate-out", "Gate A", "07:02", "Agus Wijaya",    "Seal mismatch — re-inspection 14:00", "REJECTED", "Seal mismatch"),
    Rec("2404", "OOLU 6639201", "B 9033 MVN", "Gate-in",  "Gate C", "07:15", "Dewi Lestari",   "",                                    "CLEARED",  "Reefer cargo"),
    Rec("2405", "TGHU 1178450", "B 8890 RSA", "Gate-in",  "Gate B", "07:48", "Eko Prasetyo",   "Late arrival, rescheduled slot 09:30","PENDING",  "Slot 09:30"),
    Rec("2406", "CSQU 3054218", "B 9322 KLW", "Gate-out", "Gate A", "08:05", "Rina Halim",     "",                                    "CLEARED",  ""),
    Rec("2407", "EGHU 2987610", "B 8540 TDR", "Gate-in",  "Gate B", "08:22", "Joko Susilo",    "Reefer — set to -18C on arrival",     "PENDING",  "Reefer"),
    Rec("2408", "GLDU 7702345", "B 9176 BHT", "Gate-in",  "Gate C", "08:41", "Maya Puspita",   "",                                    "CLEARED",  ""),
    Rec("2409", "BEAU 4159872", "B 8934 GJT", "Gate-out", "Gate B", "09:03", "Hendra Gunawan", "Damage noted on left door",           "REJECTED", "Damage on left door"),
    Rec("2410", "SEGU 9305617", "B 9205 CVB", "Gate-in",  "Gate A", "09:27", "Lia Kartika",    "",                                    "CLEARED",  ""),
    Rec("2411", "TRIU 2014678", "B 8602 NMK", "Gate-in",  "Gate B", "09:52", "Fajar Nugroho",  "Waiting for driver documents",        "PENDING",  "Documents pending"),
    Rec("2412", "CMAU 1480953", "B 9411 PLM", "Gate-out", "Gate C", "10:14", "Sari Wulandari", "Empty return — no cargo",             "CLEARED",  "Empty return"),
    Rec("2413", "EITU 6847219", "B 8826 HGF", "Gate-in",  "Gate A", "10:38", "Rudi Hartono",   "Overweight — rerouted to scale 2",    "REJECTED", "Overweight"),
    Rec("2414", "KKTU 5093184", "B 9158 QWD", "Gate-in",  "Gate C", "11:02", "Indah Permata",  "",                                    "CLEARED",  ""),
]

STATUS_CLASS = {"CLEARED": "text-emerald-600", "PENDING": "text-amber-500", "REJECTED": "text-red-500"}

ROW_UNSEL = "vv-row flex justify-between items-center rounded-md border px-3 py-2 mt-2 bg-white border-slate-200"
# The selected branch carries `vv-row-selected`: the author rule
# `.vv-row.vv-row-selected` (app.css, after the hover rule) keeps the
# selected paint winning over `.vv-row:hover` while the pointer rests on
# the just-clicked row (corrective-1, operator finding 2).
ROW_SEL = "vv-row vv-row-selected flex justify-between items-center rounded-md border px-3 py-2 mt-2 bg-indigo-50 border-indigo-400"
BTN_SAVED = "mt-3 w-full rounded-md px-3 py-2 text-sm font-medium bg-slate-200 text-slate-400"
# Blocked (invalid input or in-flight save): visibly unavailable — flat
# gray like saved PLUS a visible border, driven by the SAME predicate as
# :disabled so appearance and activation cannot disagree (corrective-1,
# operator finding 3). Hover adds nothing (no button hover rule exists).
BTN_BLOCKED = "mt-3 w-full rounded-md px-3 py-2 text-sm font-medium bg-slate-100 text-slate-400 border border-slate-300"
BTN_ACTIVE = "mt-3 w-full rounded-md px-3 py-2 text-sm font-medium bg-indigo-600 text-white"


def haystack(r: Rec) -> str:
    # Search indexes the registered fields as displayed (transaction,
    # direction, gate, container, truck, status); the driver name is
    # appended live so a corrected name is findable immediately (notes
    # are not indexed — documented v0 limitation).
    parts = [f"gt-{r.suffix}", r.direction.lower(), r.lane.lower(),
             r.container.lower(), r.plate.lower(), r.status.lower()]
    return "'" + " ".join(parts) + "'"


def predicate(r: Rec) -> str:
    return f"(search === '' || ({haystack(r)} + name_{r.suffix}.toLowerCase()).includes(search.toLowerCase()))"


def esc_js(s: str) -> str:
    # State values are authored without ' \ or newlines; keep it that way.
    assert not any(c in s for c in "'\\\n\r\t"), f"seed value needs escaping: {s}"
    return s


def state_block() -> str:
    lines = [
        "      selected: 'gt-2401',",
        "      search: '',",
        "      store_note: 'Dev mode — edits stay in memory (velqu-lab)',",
    ]
    for r in RECORDS:
        lines.append(f"      name_{r.suffix}: '{esc_js(r.driver)}',")
        lines.append(f"      notes_{r.suffix}: '{esc_js(r.notes)}',")
        lines.append(f"      saved_{r.suffix}: true,")
        lines.append(f"      pending_{r.suffix}: false,")
        lines.append(f"      error_{r.suffix}: '',")
        lines.append(f"      saved_at_{r.suffix}: '',")
    return "{\n" + "\n".join(lines) + "\n    }"


def rows_html() -> str:
    out = []
    for r in RECORDS:
        out.append(f'''          <div
            id="row-gt-{r.suffix}"
            data-vv-test="row-gt-{r.suffix}"
            @click="selected = 'gt-{r.suffix}'"
            vx-show="{predicate(r)}"
            :class="selected === 'gt-{r.suffix}' ? '{ROW_SEL}' : '{ROW_UNSEL}'"
          >
            <p class="text-sm font-semibold text-slate-800">GT-{r.suffix} · {r.lane}</p>
            <p class="text-xs font-bold {STATUS_CLASS[r.status]}">{r.status}</p>
          </div>''')
    return "\n".join(out)


def no_results() -> str:
    terms = " || ".join(predicate(r) for r in RECORDS)
    return f"search !== '' && !({terms})"


def shown_sum() -> str:
    return " + ".join(f"({predicate(r)} ? 1 : 0)" for r in RECORDS)


def unsaved_sum() -> str:
    return " + ".join(f"(saved_{r.suffix} ? 0 : 1)" for r in RECORDS)


def details_html() -> str:
    out = []
    for r in RECORDS:
        reason = f" · {r.reason}" if r.reason else ""
        out.append(f'''          <div data-vv-test="detail-{r.suffix}" vx-show="selected === 'gt-{r.suffix}'" class="bg-white rounded-lg border p-4">
            <p class="text-lg font-bold text-slate-900">GT-{r.suffix}</p>
            <p data-vv-test="detail-meta-{r.suffix}" class="text-xs text-slate-500 mt-1">{r.direction} · {r.lane} · {r.time} · Container {r.container}{reason}</p>
            <p class="text-xs text-slate-500 mt-1">Truck {r.plate} · {r.status}</p>
            <label class="mt-4 block text-xs text-slate-500" for="edit-name-gt-{r.suffix}">Driver name</label>
            <input
              id="edit-name-gt-{r.suffix}"
              data-vv-test="edit-name"
              vx-model="name_{r.suffix}"
              @input="saved_{r.suffix} = false; pending_{r.suffix} = false; error_{r.suffix} = ''"
              class="vv-input mt-1 w-full rounded-md border border-slate-300 px-3 py-2 text-sm text-slate-800"
            >
            <p data-vv-test="name-error-{r.suffix}" vx-show="name_{r.suffix}.trim() === '' || Array.from(name_{r.suffix}.trim()).length > 80" class="mt-1 text-xs text-red-500">Driver name is required (1-80 characters).</p>
            <label class="mt-3 block text-xs text-slate-500" for="edit-notes-gt-{r.suffix}">Notes</label>
            <textarea
              id="edit-notes-gt-{r.suffix}"
              data-vv-test="edit-notes"
              vx-model="notes_{r.suffix}"
              @input="saved_{r.suffix} = false; pending_{r.suffix} = false; error_{r.suffix} = ''"
              class="vv-input mt-1 w-full h-20 rounded-md border border-slate-300 px-3 py-2 text-sm text-slate-800"
            ></textarea>
            <p data-vv-test="notes-error-{r.suffix}" vx-show="Array.from(notes_{r.suffix}).length > 280" class="mt-1 text-xs text-red-500" vx-text="'Notes must be 280 characters or fewer (currently ' + Array.from(notes_{r.suffix}).length + ').'">Notes must be 280 characters or fewer.</p>
            <button
              id="save-gt-{r.suffix}"
              data-vv-test="save"
              @click="pending_{r.suffix} = true"
              :disabled="saved_{r.suffix} || pending_{r.suffix} || name_{r.suffix}.trim() === '' || Array.from(name_{r.suffix}.trim()).length > 80 || Array.from(notes_{r.suffix}).length > 280"
              :class="saved_{r.suffix} ? '{BTN_SAVED}' : ((pending_{r.suffix} || name_{r.suffix}.trim() === '' || Array.from(name_{r.suffix}.trim()).length > 80 || Array.from(notes_{r.suffix}).length > 280) ? '{BTN_BLOCKED}' : '{BTN_ACTIVE}')"
            >Save to device</button>
            <p data-vv-test="pending-{r.suffix}" vx-show="pending_{r.suffix}" class="mt-2 text-xs text-amber-600">Saving — the app host writes records.json to the data directory.</p>
            <p data-vv-test="saved-{r.suffix}" vx-show="saved_{r.suffix} && !pending_{r.suffix} && saved_at_{r.suffix} !== ''" class="mt-2 text-xs text-emerald-600" vx-text="'Saved to device' + (saved_at_{r.suffix} === '' ? '' : ' at ' + saved_at_{r.suffix}) + ' — survives restart.'">Saved to device — survives restart.</p>
            <p data-vv-test="save-error-{r.suffix}" vx-show="error_{r.suffix} !== ''" class="mt-2 text-xs text-red-600" vx-text="error_{r.suffix}"></p>
          </div>''')
    return "\n".join(out)


HTML = f'''<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <title>Gate Transactions — velqu-view-starter</title>
</head>
<body class="bg-slate-100">
  <!--
    Operations workflow: gate-transaction review & correction. Same
    reactive idiom as the reference dashboard (one vx-state scope,
    authored rows + per-record detail panels, vx-show filtering; vx-for
    is deferred in reactive profile v0). The vx-state attribute below
    is the ONLY part the release host rewrites: it parses the seed
    block (strict one-pair-per-line format — see src/ops/state.rs),
    applies persisted overrides from records.json, and reloads the
    document. In dev mode (velqu-lab) the seed values are used as-is
    and edits stay in memory.
  -->
  <main
    vx-state="{state_block()}"
    class="flex h-full"
  >
    <!-- Context rail -->
    <aside class="w-64 h-full bg-slate-900 p-4">
      <h1 class="text-xl font-bold text-white p-2">Gate Ops</h1>
      <p class="text-xs text-slate-500 px-2 mt-1">Port Kalibaru · Terminal 3</p>
      <p class="text-xs text-slate-400 bg-slate-800 rounded-md px-3 py-2 mt-3 font-semibold">Transactions</p>
      <p class="text-xs text-slate-600 px-2 mt-8">velqu-view-starter</p>
      <p data-vv-test="store-note" class="text-xs text-slate-500 px-2 mt-2" vx-text="store_note">Dev mode — edits stay in memory (velqu-lab)</p>
    </aside>

    <!-- Transactions: summary + searchable records + detail/edit -->
    <section data-vv-test="section-transactions" class="flex-1 p-8 h-full">
      <header class="flex justify-between items-center">
        <h2 class="text-2xl font-bold text-slate-900">Gate transactions</h2>
        <div>
          <label for="search" class="block text-xs text-slate-500 text-right">Search transactions</label>
          <input
            id="search"
            data-vv-test="search"
            vx-model="search"
            class="vv-input mt-1 w-72 rounded-md border border-slate-300 bg-white px-3 py-2 text-sm text-slate-800"
          >
        </div>
      </header>
      <p class="text-xs text-slate-500 mt-1">Search by transaction, gate, direction, container, truck, status, or driver. Find a record, correct its driver name or notes, then save to the device.</p>

      <div class="grid grid-cols-3 gap-4 mt-4">
        <div class="bg-white rounded-lg p-4 border">
          <p class="text-xs text-slate-500">Records shown</p>
          <p
            data-vv-test="summary-shown"
            class="text-3xl font-bold text-slate-900 mt-2"
            vx-text="{shown_sum()}"
          >14</p>
        </div>
        <div class="bg-white rounded-lg p-4 border">
          <p class="text-xs text-slate-500">Unsaved edits</p>
          <p
            data-vv-test="summary-unsaved"
            class="text-3xl font-bold text-amber-600 mt-2"
            vx-text="{unsaved_sum()}"
          >0</p>
        </div>
        <div class="bg-white rounded-lg p-4 border">
          <p class="text-xs text-slate-500">Pending review</p>
          <p data-vv-test="summary-pending" class="text-3xl font-bold text-slate-900 mt-2">3</p>
        </div>
      </div>

      <div class="flex mt-6 items-start">
        <!-- Records list (authored rows; vx-show filtering) -->
        <div class="flex-1 h-80 overflow-y-auto bg-white rounded-lg border mr-6 p-3">
{rows_html()}
          <div
            data-vv-test="no-results"
            vx-show="{no_results()}"
            class="rounded-md border border-slate-300 px-3 py-6 mt-4"
          >
            <p class="text-sm font-medium text-slate-500 text-center">No transactions match this search.</p>
            <p class="text-xs text-slate-400 text-center mt-1">Clear the search box to see all fourteen records.</p>
          </div>
        </div>

        <!-- Detail + edit panel (authored per record; read-only fields
             first, then the two permitted editable fields) -->
        <div class="w-96">
{details_html()}
        </div>
      </div>
    </section>
  </main>
</body>
</html>
'''

if __name__ == "__main__":
    import pathlib
    out = pathlib.Path(__file__).resolve().parent.parent / "ops" / "index.html"
    out.parent.mkdir(exist_ok=True)
    out.write_text(HTML, encoding="utf-8")
    print(f"wrote {out} ({len(HTML)} bytes, {len(RECORDS)} records)")
