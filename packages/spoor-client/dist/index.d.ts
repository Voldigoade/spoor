/**
 * Spoor TypeScript Integration Client
 * Author: Voldigoade
 * Repository: https://github.com/Voldigoade/spoor
 */
export type AnalysisMode = 'fast' | 'deep';
export type LookupStatus = 'pending' | 'running' | 'completed' | 'failed';
export interface LookupMeta {
    id: string;
    target: string;
    mode: AnalysisMode;
    status: LookupStatus;
    created_at: string;
    error?: string;
}
export type SpoorEventType = 'lookup.started' | 'target.resolved' | 'provider.started' | 'provider.completed' | 'provider.warning' | 'scan.progress' | 'analysis.completed' | 'lookup.failed';
export interface SpoorEventData {
    target?: string;
    mode?: string;
    target_type?: string;
    provider?: string;
    operation?: string;
    details?: string;
    message?: string;
    current?: number;
    total?: number;
    duration_ms?: number;
    error?: string;
}
export interface SpoorEvent {
    event: SpoorEventType;
    data: SpoorEventData;
}
export interface GeneratorInfo {
    name: string;
    version: string;
    author: string;
    repository: string;
}
export interface ReportSummary {
    display_name: string;
    target_type: string;
    account_created?: string;
    total_public_repositories: number;
    total_stars_accumulated: number;
    total_forks_accumulated: number;
    primary_languages: Array<[string, number]>;
    distinct_emails_observed: number;
    public_keys_found: number;
    fork_ratio: number;
    archived_ratio: number;
}
export interface ProfileReport {
    login: string;
    id: number;
    node_id?: string;
    name?: string;
    bio?: string;
    company?: string;
    location?: string;
    blog_url?: string;
    twitter_username?: string;
    public_email?: string;
    avatar_url: string;
    html_url: string;
    public_repos: number;
    public_gists: number;
    followers: number;
    following: number;
    created_at: string;
    updated_at: string;
    account_age_days: number;
}
export interface IdentityRecord {
    name?: string;
    email: string;
    email_type: string;
    first_seen?: string;
    last_seen?: string;
    commits_observed: number;
    repositories: string[];
    sources: string[];
    is_verified_signer: boolean;
}
export interface RepoReport {
    name: string;
    full_name: string;
    description?: string;
    is_fork: boolean;
    is_archived: boolean;
    default_branch: string;
    stars: number;
    forks: number;
    open_issues: number;
    size_kb: number;
    primary_language?: string;
    created_at?: string;
    updated_at?: string;
    pushed_at?: string;
    topics: string[];
    html_url: string;
}
export interface KeyRecord {
    key_type: 'gpg' | 'ssh';
    key_id?: string;
    fingerprint?: string;
    raw_preview: string;
    emails: string[];
    created_at?: string;
    expires_at?: string;
    can_sign?: boolean;
    source: string;
}
export interface InferenceRecord {
    topic: string;
    inference: string;
    confidence: string;
    basis: string;
    classification: string;
    limitation: string;
}
export interface CoverageRecord {
    commits_scanned: number;
    repositories_scanned: number;
    repositories_total: number;
    is_partial: boolean;
    notes?: string;
}
export interface ProviderStatus {
    status: 'ok' | 'cached' | 'partial' | 'rate_limited' | 'unavailable' | 'skipped';
    latency_ms?: number;
    items_retrieved?: number;
    message?: string;
}
export interface Report {
    schema_version: string;
    generator: GeneratorInfo;
    target: {
        type: string;
        username?: string;
        owner?: string;
        repo?: string;
    };
    mode: AnalysisMode;
    generated_at: string;
    summary: ReportSummary;
    profile?: ProfileReport;
    repositories: RepoReport[];
    identities: IdentityRecord[];
    keys: KeyRecord[];
    inferences: InferenceRecord[];
    provider_status: Record<string, ProviderStatus>;
    coverage: CoverageRecord;
    limitations: string[];
}
export type ExportFormat = 'json' | 'md' | 'txt' | 'svg' | 'pdf';
export declare class SpoorClient {
    private readonly baseUrl;
    constructor(options?: {
        baseUrl?: string;
    });
    health(): Promise<{
        status: string;
        version: string;
    }>;
    startLookup(target: string, mode?: AnalysisMode): Promise<LookupMeta>;
    getLookup(id: string): Promise<LookupMeta>;
    getReport(id: string): Promise<Report>;
    getExportUrl(id: string, format: ExportFormat): string;
    streamEvents(id: string, onEvent: (event: SpoorEvent) => void, onError?: (err: Event) => void): () => void;
}
