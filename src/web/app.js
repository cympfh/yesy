const form = document.querySelector('#search-form');
const input = document.querySelector('#query');
const status = document.querySelector('#status');
const results = document.querySelector('#results');
const template = document.querySelector('#result-template');
const historySection = document.querySelector('#search-history');
const historyList = document.querySelector('#search-history-list');
const voiceButton = document.querySelector('#voice-search');
const HISTORY_KEY = 'yesy.search-history.v1';
const HISTORY_REPLACEMENT_WINDOW = 8000;
let searchHistory = loadSearchHistory();
let controller;
let previousSearch;

function loadSearchHistory() {
  try {
    const saved = JSON.parse(localStorage.getItem(HISTORY_KEY) || '[]');
    if (!Array.isArray(saved)) return [];
    const queries = saved.filter((query) => typeof query === 'string' && query.trim());
    return [...new Set(queries)].slice(0, 30);
  } catch {
    return [];
  }
}

function recordSearchHistory(query) {
  const searchedAt = Date.now();
  if (previousSearch && (
    query.startsWith(previousSearch.query)
    || searchedAt - previousSearch.searchedAt <= HISTORY_REPLACEMENT_WINDOW
  )) {
    searchHistory = searchHistory.filter((item) => item !== previousSearch.query);
  }
  searchHistory = [query, ...searchHistory.filter((item) => item !== query)].slice(0, 30);
  previousSearch = { query, searchedAt };
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(searchHistory));
  } catch {
    return;
  }
}

function renderSearchHistory() {
  historyList.replaceChildren();
  historySection.hidden = searchHistory.length === 0;
  for (const query of searchHistory) {
    const link = document.createElement('a');
    link.href = `/?q=${encodeURIComponent(query)}`;
    link.textContent = query;
    historyList.append(link);
  }
}

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
      const openWith = item.querySelector('.open-with');
      openWith.hidden = false;
      for (const button of openWith.querySelectorAll('.open-application')) {
        button.dataset.path = result.path;
      }
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

results.addEventListener('click', async (event) => {
  const button = event.target.closest('.open-application');
  if (!button) return;
  const feedback = button.closest('.open-with').querySelector('.open-feedback');
  button.disabled = true;
  feedback.textContent = 'Opening...';
  try {
    const response = await fetch('/api/open', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ path: button.dataset.path, application: button.dataset.application }),
    });
    const data = response.status === 204 ? null : await response.json();
    if (!response.ok) throw new Error(data?.error || 'The file could not be opened.');
    feedback.textContent = 'Opened.';
  } catch (error) {
    feedback.textContent = error.message;
  } finally {
    button.disabled = false;
  }
});

async function search() {
  controller?.abort();
  const query = input.value.trim();
  const url = new URL(location.href);
  if (query) {
    url.searchParams.set('q', query);
  } else {
    url.searchParams.delete('q');
  }
  history.replaceState(null, '', url);
  if (!query) {
    controller = undefined;
    results.replaceChildren();
    status.textContent = '';
    renderSearchHistory();
    return;
  }
  recordSearchHistory(query);
  historySection.hidden = true;
  const currentController = new AbortController();
  controller = currentController;
  status.textContent = 'Searching…';
  try {
    const response = await fetch(`/api/search?q=${encodeURIComponent(query)}`, {
      signal: currentController.signal,
    });
    const data = await response.json();
    if (currentController.signal.aborted) return;
    if (!response.ok) throw new Error(data.error || 'Search failed.');
    render(data);
  } catch (error) {
    if (error.name === 'AbortError' || currentController.signal.aborted) return;
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
  controller?.abort();
  clearTimeout(timer);
  timer = setTimeout(search, 180);
});

const SpeechRecognition = window.SpeechRecognition || window.webkitSpeechRecognition;
if (SpeechRecognition) {
  const recognition = new SpeechRecognition();
  recognition.lang = navigator.language;
  recognition.interimResults = true;
  recognition.continuous = false;
  let shouldSearch = false;

  voiceButton.hidden = false;
  voiceButton.addEventListener('click', () => {
    if (voiceButton.getAttribute('aria-pressed') === 'true') {
      recognition.stop();
      return;
    }
    shouldSearch = false;
    recognition.start();
  });

  recognition.addEventListener('start', () => {
    voiceButton.setAttribute('aria-pressed', 'true');
    voiceButton.setAttribute('aria-label', 'Stop voice input');
    voiceButton.title = 'Stop voice input';
    status.textContent = 'Listening…';
  });

  recognition.addEventListener('result', (event) => {
    let transcript = '';
    for (let index = 0; index < event.results.length; index += 1) {
      transcript += event.results[index][0].transcript;
      if (event.results[index].isFinal) shouldSearch = true;
    }
    input.value = transcript.trim();
  });

  recognition.addEventListener('error', (event) => {
    shouldSearch = false;
    status.textContent = event.error === 'not-allowed'
      ? 'Microphone access was blocked. Allow access in your browser settings and try again.'
      : 'Voice input failed. Try again or type your search.';
  });

  recognition.addEventListener('end', () => {
    voiceButton.setAttribute('aria-pressed', 'false');
    voiceButton.setAttribute('aria-label', 'Search by voice');
    voiceButton.title = 'Search by voice';
    if (shouldSearch && input.value.trim()) search();
  });
}

renderSearchHistory();
const initialQuery = new URLSearchParams(location.search).get('q');
if (initialQuery) {
  input.value = initialQuery;
  search();
}
