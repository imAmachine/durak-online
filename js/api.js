export async function requestJSON(path, body, errorMessage) {
  const response = await fetch(path, body ? { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) } : undefined);
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || errorMessage);
  return data;
}

export function createApi(getToken) {
  return (path, body) => requestJSON(path, body && { ...body, ...(body.clientId ? { token: getToken() } : {}) }, 'Не удалось выполнить действие');
}
