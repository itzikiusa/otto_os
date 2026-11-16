// The open IDE project's name for the window/tab title (review I3-13): "<project> — IDE —
// Otto". A dependency-free holder so the app shell can read it without loading the IDE.

class IdeWindowTitle {
  project = $state('');
}

export const ideWindowTitle = new IdeWindowTitle();
