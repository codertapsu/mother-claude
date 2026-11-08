import { Component, computed, inject, signal } from '@angular/core';

import { ApiService, errorMessage } from '../../core/api.service';
import {
  REPORT_DAYS,
  ReportRow,
  RoutingReport,
  choiceLabel,
  csvName,
  pctLabel,
  projectName,
  ratioLabel,
} from '../../core/capacity';
import { tr } from '../../core/i18n/i18n';
import { TPipe } from '../../core/i18n/t.pipe';
import { CopyButtonComponent } from '../../shared/copy-button.component';
import { IconComponent } from '../../shared/icon.component';
import { TimeAgoComponent } from '../../shared/time-ago.component';
import { formatTokens, formatUsd } from '../../shared/util';

/** What tasks cost and how they ended, by class × model × effort; CSV. */
@Component({
  selector: 'mc-routing-report',
  imports: [CopyButtonComponent, IconComponent, TimeAgoComponent, TPipe],
  template: `
    <div class="row head">
      <div class="segmented" [attr.aria-label]="'Period' | t">
        @for (d of periods; track d) {
          <button
            type="button"
            [attr.aria-pressed]="days() === d"
            [class.active]="days() === d"
            (click)="setDays(d)"
          >
            {{ '{n} days' | t: { n: d } }}
          </button>
        }
      </div>
      <span class="spacer"></span>
      <button class="btn small" type="button" [disabled]="exporting()" (click)="download('groups')">
        <mc-icon name="download" [size]="14" /> CSV
      </button>
      <button
        class="btn small"
        type="button"
        [disabled]="exporting()"
        (click)="download('outcomes')"
      >
        <mc-icon name="download" [size]="14" /> {{ 'Every task' | t }}
      </button>
      @if (csv()) {
        <mc-copy-button [text]="csv()" [label]="'Copy CSV' | t" [ariaLabel]="'Copy the CSV' | t" />
      }
    </div>

    @if (error()) {
      <p class="notice error" role="alert">{{ error() }}</p>
    }

    @if (report(); as r) {
      <p class="muted small">{{ r.estimateNote }}</p>
      @if (r.pending) {
        <p class="muted small">
          {{
            r.pending === 1
              ? ('{n} task end is still being read.' | t: { n: r.pending })
              : ('{n} task ends are still being read.' | t: { n: r.pending })
          }}
        </p>
      }
      @if (!r.totals.tasks) {
        <p class="empty-state">
          <strong>{{ 'No task ended in the last {n} days' | t: { n: r.days } }}</strong>
          {{ 'Reports fill in as teams finish tasks.' | t }}
        </p>
      } @else {
        <div class="tiles">
          <div class="card tile">
            <span class="field-label">{{ 'Tasks ended' | t }}</span>
            <span class="big">{{ r.totals.tasks }}</span>
            <span class="muted small">{{ '{n} completed' | t: { n: r.totals.completed } }}</span>
          </div>
          <div class="card tile">
            <span class="field-label">{{ 'Cost (estimated)' | t }}</span>
            <span class="big">{{ usd(r.totals.costUsd) }}</span>
            <span class="muted small">{{
              '{usd} per completed task' | t: { usd: usd(r.totals.costPerCompletedUsd) }
            }}</span>
          </div>
          <div class="card tile">
            <span class="field-label">{{ 'Escalations||rate' | t }}</span>
            <span class="big">{{ ratio(r.totals.escalationRate) }}</span>
            <span class="muted small">{{
              'gates passed {pct}' | t: { pct: ratio(r.totals.gatePassRate) }
            }}</span>
          </div>
          <div class="card tile">
            <span class="field-label">{{ 'Cache hits' | t }}</span>
            <span class="big">{{ ratio(r.totals.cacheHitRatio) }}</span>
            <span class="muted small">{{ 'of input tokens' | t }}</span>
          </div>
          @if (r.totals.windowPctPerCompleted !== undefined) {
            <div class="card tile">
              <span class="field-label">{{ '5-hour window' | t }}</span>
              <span class="big">{{ pct(r.totals.windowPctPerCompleted) }}</span>
              <span class="muted small">{{
                'per completed task (plan accounts, estimated)' | t
              }}</span>
            </div>
          }
        </div>

        <h2>{{ 'By class' | t }}</h2>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th scope="col">{{ 'Class' | t }}</th>
                <th scope="col" class="n">{{ 'Completed' | t }}</th>
                <th scope="col" class="n">{{ 'Est. cost' | t }}</th>
                <th scope="col" class="n">{{ 'Per completed' | t }}</th>
                <th scope="col" class="n">{{ 'Escalated' | t }}</th>
                <th scope="col" class="n">{{ 'Gates passed' | t }}</th>
              </tr>
            </thead>
            <tbody>
              @for (c of r.classes; track c.class) {
                <tr>
                  <th scope="row">{{ c.class }}</th>
                  <td class="n">{{ c.completed }} / {{ c.tasks }}</td>
                  <td class="n">{{ usd(c.costUsd) }}</td>
                  <td class="n">{{ usd(c.costPerCompletedUsd) }}</td>
                  <td class="n">{{ ratio(c.escalationRate) }}</td>
                  <td class="n">{{ ratio(c.gatePassRate) }}</td>
                </tr>
              }
            </tbody>
          </table>
        </div>

        <h2>{{ 'By class, model and effort' | t }}</h2>
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th scope="col">{{ 'Class' | t }}</th>
                <th scope="col">{{ 'Model' | t }}</th>
                <th scope="col" class="n">{{ 'Completed' | t }}</th>
                <th scope="col" class="bar-col">{{ 'Cost per completed task' | t }}</th>
                <th scope="col" class="n">{{ 'Tokens each' | t }}</th>
                <th scope="col" class="n">{{ 'Cache hits' | t }}</th>
                <th scope="col" class="n">{{ 'Escalated' | t }}</th>
              </tr>
            </thead>
            <tbody>
              @for (x of r.rows; track x.class + x.model + x.effort) {
                <tr>
                  <th scope="row">{{ x.class }}</th>
                  <td>{{ rowChoice(x) }}</td>
                  <td class="n">{{ x.completed }} / {{ x.tasks }}</td>
                  <td class="bar-col">
                    <span class="cost-bar">
                      <span class="fill" [style.width.%]="share(x)"></span>
                    </span>
                    <span class="n">{{ usd(x.costPerCompletedUsd) }}</span>
                  </td>
                  <td class="n">{{ tokens(x.tokensPerCompleted) }}</td>
                  <td class="n">{{ ratio(x.cacheHitRatio) }}</td>
                  <td class="n">{{ ratio(x.escalationRate) }}</td>
                </tr>
              }
            </tbody>
          </table>
        </div>

        @if (r.escalations.length) {
          <h2>{{ 'Escalations in the period' | t }}</h2>
          <ul class="card list plain">
            @for (n of r.escalations; track n.at + n.taskId) {
              <li class="list-row small">
                <span class="grow"
                  >{{ n.direction === 'up' ? '↑' : '↓' }}
                  {{
                    '{task} in {project}: {reason}'
                      | t: { task: n.taskId, project: project(n.project), reason: n.reason }
                  }}</span
                >
                <span class="muted"><mc-time [ms]="n.at" /></span>
              </li>
            }
          </ul>
        }
        @if (r.classifier.calls) {
          <p class="muted small">
            {{
              'The Haiku classifier: {calls} calls, {usd} (estimated).'
                | t: { calls: r.classifier.calls, usd: usd(r.classifier.costUsd) }
            }}
          </p>
        }
      }
    } @else if (loading()) {
      <p class="muted">{{ 'Loading…' | t }}</p>
    }
  `,
  styles: [
    `
      .head {
        margin-bottom: 0.75rem;
      }
      .tiles {
        display: grid;
        grid-template-columns: repeat(auto-fit, minmax(10rem, 1fr));
        gap: 0.5rem;
      }
      .tiles .card + .card {
        margin-top: 0;
      }
      .tile {
        display: flex;
        flex-direction: column;
        gap: 0.2rem;
        min-width: 0;
      }
      .big {
        font-size: 1.35rem;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
      }
      .table-wrap {
        overflow-x: auto;
        border: 1px solid var(--border);
        border-radius: var(--radius);
        background: var(--panel);
      }
      table {
        width: 100%;
        border-collapse: collapse;
        font-size: 0.85rem;
      }
      th,
      td {
        padding: 0.45rem 0.6rem;
        text-align: left;
        border-bottom: 1px solid var(--border);
        white-space: nowrap;
      }
      thead th {
        color: var(--muted);
        font-weight: 600;
        font-size: 0.75rem;
      }
      tbody tr:last-child th,
      tbody tr:last-child td {
        border-bottom: 0;
      }
      .n {
        text-align: right;
        font-variant-numeric: tabular-nums;
      }
      .bar-col {
        min-width: 10rem;
      }
      td.bar-col {
        display: flex;
        align-items: center;
        gap: 0.5rem;
      }
      .cost-bar {
        flex: 1 1 auto;
        height: 0.4rem;
        border-radius: 0.2rem;
        background: var(--panel-3);
        overflow: hidden;
      }
      .cost-bar .fill {
        display: block;
        height: 100%;
        background: var(--accent);
      }
      .list {
        padding-top: 0;
        padding-bottom: 0;
      }
      .plain {
        list-style: none;
        margin: 0;
      }
      .grow {
        flex: 1 1 auto;
        min-width: 0;
        overflow-wrap: anywhere;
      }
      h2 {
        margin-top: 1.5rem;
      }
    `,
  ],
})
export class RoutingReportComponent {
  private api = inject(ApiService);

  protected readonly periods = REPORT_DAYS;
  protected readonly days = signal<number>(30);
  protected readonly report = signal<RoutingReport | null>(null);
  protected readonly error = signal('');
  protected readonly loading = signal(false);
  protected readonly exporting = signal(false);
  /** The last CSV fetched, for the copy button. */
  protected readonly csv = signal('');
  private readonly maxCost = computed(() =>
    Math.max(0, ...(this.report()?.rows ?? []).map((r) => r.costPerCompletedUsd ?? 0)),
  );

  protected readonly usd = formatUsd;
  protected readonly tokens = formatTokens;
  protected readonly ratio = ratioLabel;
  protected readonly pct = pctLabel;
  protected readonly project = projectName;

  constructor() {
    void this.load();
  }

  /** A row's cost per completed task against the dearest one (bar width). */
  protected share(r: ReportRow): number {
    const max = this.maxCost();
    return max > 0 ? Math.round(((r.costPerCompletedUsd ?? 0) / max) * 100) : 0;
  }

  /** "Sonnet · medium"; "Sonnet" when the effort was the model's default. */
  protected rowChoice(r: ReportRow): string {
    return choiceLabel(
      r.effort === 'default' ? { model: r.model } : { model: r.model, effort: r.effort },
    );
  }

  protected setDays(d: number): void {
    this.days.set(d);
    this.csv.set('');
    void this.load();
  }

  async load(): Promise<void> {
    this.loading.set(true);
    try {
      this.report.set(await this.api.getRoutingReport(this.days()));
      this.error.set('');
    } catch (e) {
      this.error.set(tr('Could not load the report: {error}', { error: errorMessage(e) }));
    } finally {
      this.loading.set(false);
    }
  }

  /** Fetch the CSV (with this client's credentials) and save it; it also
   * stays ready to copy where a download is not possible. */
  async download(rows: 'groups' | 'outcomes'): Promise<void> {
    this.exporting.set(true);
    try {
      const text = await this.api.getRoutingReportCsv(this.days(), rows);
      this.csv.set(text);
      const url = URL.createObjectURL(new Blob([text], { type: 'text/csv' }));
      const a = document.createElement('a');
      a.href = url;
      a.download = csvName(this.days(), rows);
      a.rel = 'noopener';
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 10_000);
      this.error.set('');
    } catch (e) {
      this.error.set(tr('Could not export the report: {error}', { error: errorMessage(e) }));
    } finally {
      this.exporting.set(false);
    }
  }
}
