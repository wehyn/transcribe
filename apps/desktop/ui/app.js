const consent = document.querySelector('#consent');
const recordButton = document.querySelector('#record');
const livePanel = document.querySelector('#live-panel');
const titleInput = document.querySelector('#title');
const liveTitle = document.querySelector('#live-title');
const timer = document.querySelector('#timer');
const pauseButton = document.querySelector('#pause');
const stopButton = document.querySelector('#stop');
let startedAt = null;
let timerId = null;
let paused = false;

const setCapabilityStatus = (id, available, fallback) => {
  const element = document.querySelector(id);
  element.textContent = available ? 'Available · opens on Record' : fallback;
  element.classList.toggle('available', available);
};

// The static shell cannot inspect native devices. The Rust command surface
// supplies the same metadata without opening a stream when wired into Tauri.
setCapabilityStatus('#mic-status', true, 'Unavailable on this Mac');
setCapabilityStatus('#system-status', true, 'Requires Screen Recording permission');

consent.addEventListener('change', () => {
  recordButton.disabled = !consent.checked;
});

const updateTimer = () => {
  if (startedAt === null || paused) return;
  const elapsed = Math.floor((Date.now() - startedAt) / 1000);
  timer.textContent = `${String(Math.floor(elapsed / 60)).padStart(2, '0')}:${String(elapsed % 60).padStart(2, '0')}`;
};

recordButton.addEventListener('click', () => {
  if (!consent.checked) return;
  liveTitle.textContent = titleInput.value.trim() || 'Untitled meeting';
  livePanel.classList.remove('hidden');
  recordButton.disabled = true;
  consent.disabled = true;
  titleInput.disabled = true;
  document.querySelector('#language').disabled = true;
  startedAt = Date.now();
  timerId = window.setInterval(updateTimer, 1000);
  window.scrollTo({ top: document.body.scrollHeight, behavior: 'smooth' });
});

pauseButton.addEventListener('click', () => {
  paused = !paused;
  pauseButton.textContent = paused ? 'Resume' : 'Pause';
  pauseButton.classList.toggle('paused', paused);
});

stopButton.addEventListener('click', () => {
  window.clearInterval(timerId);
  pauseButton.disabled = true;
  stopButton.disabled = true;
  stopButton.textContent = 'Recording stopped';
  document.querySelector('.recording-dot').style.background = '#aaa29a';
});
