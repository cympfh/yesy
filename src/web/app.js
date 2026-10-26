const form = document.querySelector('#search-form');
const input = document.querySelector('#query');
const status = document.querySelector('#status');
const results = document.querySelector('#results');
const template = document.querySelector('#result-template');
let controller;

function appendHighlightedText(element, text, query) {
  element.replaceChildren();
  if (!query) {
    element.textContent = text;
    return;
  }
  const lowerText = text.toLocaleLowerCase();
  const lowerQuery = query.toLocaleLowerCase();
  let offset = 0;
  let match = lowerText.indexOf(lowerQuery, offset);
  while (match !== -1) {
    element.append(document.createTextNode(text.slice(offset, match)));
    const mark = document.createElement('mark');
    mark.textContent = text.slice(match, match + query.length);
    element.append(mark);
    offset = match + query.length;
    match = lowerText.indexOf(lowerQuery, offset);
  }
  element.append(document.createTextNode(text.slice(offset)));
}

function fileName(path) {
  return path.split('/').filter(Boolean).pop() || path;
}

function render(data) {
  results.replaceChildren();
  for (const result of data.results) {
    const item = template.content.cloneNode(true);
    const name = fileName(result.path);
    appendHighlightedText(item.querySelector('h2'), name, data.query);
    const preview = item.querySelector('.image-preview');
    if (result.image_url) {
      preview.src = result.image_url;
      preview.alt = `Preview of ${name}`;
      preview.hidden = false;
      preview.addEventListener('error', () => preview.remove(), { once: true });
    } else {
      preview.remove();
    }
    const snippet = item.querySelector('.snippet');
    if (result.snippet) {
      appendHighlightedText(snippet, result.snippet, data.query);
    } else {
      snippet.remove();
    }
    const linuxRow = item.querySelector('.linux-row');
    linuxRow.querySelector('code').textContent = result.path;
    linuxRow.querySelector('.copy-path').dataset.path = result.path;
    const windowsRow = item.querySelector('.windows-row');
    if (result.windows_path && result.windows_url) {
      windowsRow.querySelector('a').href = result.windows_url;
      windowsRow.querySelector('code').textContent = result.windows_path;
      windowsRow.querySelector('.copy-path').dataset.path = result.windows_path;
    } else {
      windowsRow.remove();
    }
    results.append(item);
  }
  const count = data.results.length;
  status.textContent = count === 0
    ? `No files match “${data.query}”. Try a shorter phrase or run yesy update.`
    : `${count}${count === 100 ? '+' : ''} ${count === 1 ? 'file' : 'files'} found for “${data.query}”.`;
}

async function copyText(text) {
  if (navigator.clipboard?.writeText) {
    await navigator.clipboard.writeText(text);
    return;
  }
  const textarea = document.createElement('textarea');
  textarea.value = text;
  textarea.style.position = 'fixed';
  textarea.style.opacity = '0';
  document.body.append(textarea);
  textarea.select();
  const copied = document.execCommand('copy');
  textarea.remove();
  if (!copied) throw new Error('Copy failed.');
}

results.addEventListener('click', async (event) => {
  const button = event.target.closest('.copy-path');
  if (!button) return;
  const feedback = button.closest('.path-row').querySelector('.copy-feedback');
  try {
    await copyText(button.dataset.path);
    feedback.textContent = 'Copied.';
  } catch {
    feedback.textContent = 'Copy failed. Select the path and copy it manually.';
  }
});

async function search() {
  const query = input.value.trim();
  const url = new URL(location.href);
  if (query) {
    url.searchParams.set('q', query);
  } else {
    url.searchParams.delete('q');
  }
  history.replaceState(null, '', url);
  if (!query) {
    results.replaceChildren();
    status.textContent = '';
    return;
  }
  controller?.abort();
  controller = new AbortController();
  status.textContent = 'Searching…';
  try {
    const response = await fetch(`/api/search?q=${encodeURIComponent(query)}`, {
      signal: controller.signal,
    });
    const data = await response.json();
    if (!response.ok) throw new Error(data.error || 'Search failed.');
    render(data);
  } catch (error) {
    if (error.name === 'AbortError') return;
    results.replaceChildren();
    status.textContent = `${error.message} Run yesy update and try again.`;
  }
}

form.addEventListener('submit', (event) => {
  event.preventDefault();
  search();
});

let timer;
input.addEventListener('input', () => {
  clearTimeout(timer);
  timer = setTimeout(search, 180);
});

const initialQuery = new URLSearchParams(location.search).get('q');
if (initialQuery) {
  input.value = initialQuery;
  search();
}
