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
    appendHighlightedText(item.querySelector('h2'), fileName(result.path), data.query);
    const snippet = item.querySelector('.snippet');
    if (result.snippet) {
      appendHighlightedText(snippet, result.snippet, data.query);
    } else {
      snippet.remove();
    }
    item.querySelector('.linux-row code').textContent = result.path;
    const windowsRow = item.querySelector('.windows-row');
    if (result.windows_path && result.windows_url) {
      const link = windowsRow.querySelector('a');
      link.href = result.windows_url;
      link.querySelector('code').textContent = result.windows_path;
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

async function search() {
  const query = input.value.trim();
  if (!query) {
    results.replaceChildren();
    status.textContent = 'Search the files indexed by yesy.';
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
