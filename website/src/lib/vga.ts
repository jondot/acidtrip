// The 16 VGA colors, as the editor names them.
const NAMES = ['black', 'blue', 'green', 'cyan', 'red', 'magenta', 'brown', 'gray', 'dark gray', 'light blue',
  'light green', 'light cyan', 'light red', 'light magenta', 'yellow', 'white'];
const HEX = ['#000000', '#0000aa', '#00aa00', '#00aaaa', '#aa0000', '#aa00aa', '#aa5500', '#aaaaaa', '#555555',
  '#5555ff', '#55ff55', '#55ffff', '#ff5555', '#ff55ff', '#ffff55', '#ffffff'];

export function lum(hex: string): number {
  const n = parseInt(hex.slice(1), 16);
  return (0.2126 * (n >> 16) + 0.7152 * ((n >> 8) & 255) + 0.0722 * (n & 255)) / 255;
}

export const VGA = NAMES.map((name, i) => ({ name, hex: HEX[i], light: lum(HEX[i]) > 0.45 }));
