"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.SpoorClient = void 0;
class SpoorClient {
    baseUrl;
    constructor(options = {}) {
        this.baseUrl = (options.baseUrl || 'http://localhost:8080').replace(/\/+$/, '');
    }
    async health() {
        const res = await fetch(`${this.baseUrl}/healthz`);
        if (!res.ok) {
            throw new Error(`Health check failed with HTTP ${res.status}`);
        }
        return res.json();
    }
    async startLookup(target, mode = 'fast', enrich) {
        const payload = {
            target,
            mode,
        };
        if (enrich !== undefined) {
            payload.enrich = enrich;
        }
        const res = await fetch(`${this.baseUrl}/v1/lookups`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json',
            },
            body: JSON.stringify(payload),
        });
        if (!res.ok) {
            const err = await res.json().catch(() => ({ error: res.statusText }));
            throw new Error(err.error || `Lookup request failed: ${res.status}`);
        }
        return res.json();
    }
    async getLookup(id) {
        const res = await fetch(`${this.baseUrl}/v1/lookups/${id}`);
        if (!res.ok) {
            throw new Error(`Failed to fetch lookup metadata: ${res.status}`);
        }
        return res.json();
    }
    async getReport(id) {
        const res = await fetch(`${this.baseUrl}/v1/lookups/${id}/report`);
        if (res.status === 202) {
            throw new Error('Analysis still in progress');
        }
        if (!res.ok) {
            const err = await res.json().catch(() => ({ error: res.statusText }));
            throw new Error(err.error || `Report retrieval failed: ${res.status}`);
        }
        return res.json();
    }
    getExportUrl(id, format) {
        return `${this.baseUrl}/v1/lookups/${id}/export/${format}`;
    }
    streamEvents(id, onEvent, onError) {
        const url = `${this.baseUrl}/v1/lookups/${id}/events`;
        const eventSource = new EventSource(url);
        const eventNames = [
            'lookup.started',
            'target.resolved',
            'provider.started',
            'provider.completed',
            'provider.warning',
            'scan.progress',
            'analysis.completed',
            'lookup.failed',
        ];
        for (const name of eventNames) {
            eventSource.addEventListener(name, (e) => {
                try {
                    const parsed = JSON.parse(e.data);
                    onEvent({ event: name, data: parsed.data || parsed });
                }
                catch {
                    // ignore malformed frame
                }
            });
        }
        if (onError) {
            eventSource.onerror = onError;
        }
        return () => {
            eventSource.close();
        };
    }
}
exports.SpoorClient = SpoorClient;
