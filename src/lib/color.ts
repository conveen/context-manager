/**
 * Generate a hue value (0–360) based on a string hash.
 * Used to consistently color-code contexts in the sidebar.
 */
export function hueFor(id: string): number {
    let hash = 0;
    for (let i = 0; i < id.length; i++) {
        hash = ((hash << 5) - hash + id.charCodeAt(i)) | 0;
    }
    return Math.abs(hash) % 360;
}
