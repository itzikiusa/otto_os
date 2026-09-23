// Site Studio — the block library: every section block (`<family>/<variant>`)
// and child block (`item/*`, `embed/*`) with its label, inspector fields and
// starter content. Pure data; the renderers (render.ts + the Rust twin in
// crates/otto-design/src/site/render.rs) and the validators accept exactly
// these names — add a block in all four places.

import type { SiteBlock, SiteProps, SiteSection, SiteStyle } from './types';

export type FieldKind = 'text' | 'textarea' | 'url' | 'toggle' | 'icon' | 'lines' | 'links' | 'media';

export interface FieldDef {
  key: string;
  label: string;
  kind: FieldKind;
  placeholder?: string;
  hint?: string;
  /** Render at half width next to its sibling (label + link pairs). */
  half?: boolean;
}

export type Family =
  | 'nav'
  | 'hero'
  | 'features'
  | 'social'
  | 'pricing'
  | 'faq'
  | 'cta'
  | 'content'
  | 'media'
  | 'footer';

export const FAMILIES: readonly { id: Family; label: string }[] = [
  { id: 'nav', label: 'Navigation' },
  { id: 'hero', label: 'Hero' },
  { id: 'features', label: 'Features' },
  { id: 'social', label: 'Social proof' },
  { id: 'pricing', label: 'Pricing & tiers' },
  { id: 'faq', label: 'FAQ' },
  { id: 'cta', label: 'Call to action' },
  { id: 'content', label: 'Content' },
  { id: 'media', label: 'Media' },
  { id: 'footer', label: 'Footer' },
];

export interface ItemDef {
  block: string;
  label: string;
  /** The prop that names an item in Layers ("Gold", "How fast…"). */
  titleKey: string;
  fields: FieldDef[];
  defaults: SiteProps;
}

export interface SectionDef {
  block: string;
  family: Family;
  label: string;
  description: string;
  fields: FieldDef[];
  /** The child block type this section repeats (cards, tiers, rows…). */
  item?: string;
  /** Child blocks allowed in the media slot (first match renders). */
  media?: string[];
  keywords: string;
  props: SiteProps;
  style: SiteStyle;
  blocks: Omit<SiteBlock, 'id'>[];
}

// ── Child blocks ─────────────────────────────────────────────────────────────

const f = (key: string, label: string, kind: FieldKind = 'text', extra: Partial<FieldDef> = {}): FieldDef => ({
  key,
  label,
  kind,
  ...extra,
});

export const ITEMS: readonly ItemDef[] = [
  {
    block: 'item/link',
    label: 'Link',
    titleKey: 'label',
    fields: [f('label', 'Label', 'text', { half: true }), f('href', 'Link', 'url', { half: true, placeholder: '#faq or https://…' })],
    defaults: { label: 'Link', href: '#' },
  },
  {
    block: 'item/feature',
    label: 'Feature',
    titleKey: 'title',
    fields: [
      f('icon', 'Icon', 'icon', { hint: 'A named icon, or up to 3 characters (“2×”).' }),
      f('title', 'Title'),
      f('body', 'Text', 'textarea'),
      f('image', 'Image', 'media', { hint: 'Shown by the alternating and bento layouts.' }),
    ],
    defaults: { icon: 'sparkle', title: 'A clear benefit', body: 'One sentence on why it matters to the person reading.' },
  },
  {
    block: 'item/step',
    label: 'Step',
    titleKey: 'title',
    fields: [f('title', 'Title'), f('body', 'Text', 'textarea')],
    defaults: { title: 'Do the thing', body: 'What happens in this step, in one line.' },
  },
  {
    block: 'item/stat',
    label: 'Stat',
    titleKey: 'label',
    fields: [f('value', 'Value', 'text', { half: true }), f('label', 'Label', 'text', { half: true })],
    defaults: { value: '98%', label: 'of members come back' },
  },
  {
    block: 'item/logo',
    label: 'Logo',
    titleKey: 'name',
    fields: [f('name', 'Wordmark'), f('image', 'Logo image', 'media')],
    defaults: { name: 'Northwind' },
  },
  {
    block: 'item/testimonial',
    label: 'Quote',
    titleKey: 'name',
    fields: [f('quote', 'Quote', 'textarea'), f('name', 'Name', 'text', { half: true }), f('role', 'Role', 'text', { half: true })],
    defaults: { quote: 'It paid for itself in the first week.', name: 'Sam Rivera', role: 'Member since 2024' },
  },
  {
    block: 'item/tier',
    label: 'Tier',
    titleKey: 'name',
    fields: [
      f('name', 'Name', 'text', { half: true }),
      f('badge', 'Badge', 'text', { half: true, placeholder: 'Most popular' }),
      f('price', 'Price', 'text', { half: true }),
      f('period', 'Period', 'text', { half: true, placeholder: '/month' }),
      f('blurb', 'Blurb', 'textarea'),
      f('features', 'Perks (one per line)', 'lines'),
      f('cta_label', 'Button', 'text', { half: true }),
      f('cta_href', 'Button link', 'url', { half: true }),
      f('highlight', 'Highlight this tier', 'toggle'),
    ],
    defaults: {
      name: 'Plus',
      price: '$12',
      period: '/month',
      blurb: 'For people who want more.',
      features: ['Everything in Free', 'Priority support'],
      cta_label: 'Choose Plus',
      cta_href: '#',
      highlight: false,
      badge: '',
    },
  },
  {
    block: 'item/faq',
    label: 'Question',
    titleKey: 'question',
    fields: [f('question', 'Question'), f('answer', 'Answer', 'textarea')],
    defaults: { question: 'Is there a free plan?', answer: 'Yes — start free and upgrade whenever you like.' },
  },
  {
    block: 'item/column',
    label: 'Link column',
    titleKey: 'title',
    fields: [f('title', 'Title'), f('links', 'Links', 'links')],
    defaults: { title: 'Product', links: [{ label: 'Features', href: '#features' }, { label: 'Pricing', href: '#pricing' }] },
  },
  {
    block: 'embed/3d',
    label: '3D embed',
    titleKey: 'caption',
    fields: [
      f('src', '3D artifact', 'media', { hint: 'otto://design/<id>@approved — follows the approved version.' }),
      f('alt', 'Text alternative'),
      f('label', 'Placeholder label', 'text', { hint: 'Shown on the stand-in card until a 3D artifact is picked.' }),
      f('caption', 'Caption'),
      f('auto_rotate', 'Auto-rotate', 'toggle'),
      f('tilt_on_hover', 'Tilt on hover', 'toggle'),
    ],
    defaults: { src: '', alt: '', label: '', caption: '', auto_rotate: true, tilt_on_hover: true },
  },
  {
    block: 'embed/image',
    label: 'Image',
    titleKey: 'alt',
    fields: [f('src', 'Image', 'media'), f('alt', 'Text alternative'), f('caption', 'Caption')],
    defaults: { src: '', alt: '', caption: '' },
  },
];

// ── Section blocks ───────────────────────────────────────────────────────────

const HERO_FIELDS: FieldDef[] = [
  f('eyebrow', 'Eyebrow'),
  f('headline', 'Headline', 'textarea'),
  f('subhead', 'Subhead', 'textarea'),
  f('primary_label', 'Primary button', 'text', { half: true }),
  f('primary_href', 'Primary link', 'url', { half: true }),
  f('secondary_label', 'Secondary button', 'text', { half: true }),
  f('secondary_href', 'Secondary link', 'url', { half: true }),
  f('trust', 'Trust line', 'text', { placeholder: '★ 4.8 from 12,000 members' }),
];
const HERO_PROPS: SiteProps = {
  eyebrow: 'New · Launch week',
  headline: 'Ship pages people remember.',
  subhead: 'Say what you do, who it is for and why now — in two short sentences.',
  primary_label: 'Get started',
  primary_href: '#',
  secondary_label: 'See how it works',
  secondary_href: '#how',
  trust: '',
};
const HEADING_FIELDS: FieldDef[] = [f('eyebrow', 'Eyebrow'), f('heading', 'Heading', 'textarea'), f('subheading', 'Subheading', 'textarea')];
const features = (n: number): Omit<SiteBlock, 'id'>[] =>
  [
    { icon: 'bolt', title: 'Fast by default', body: 'Pages load in a blink and stay light on every device.' },
    { icon: 'shield', title: 'Private by design', body: 'Your data stays yours. No trackers, no surprises.' },
    { icon: 'heart', title: 'Made to delight', body: 'Small details that make people smile — and come back.' },
    { icon: 'chart', title: 'Measurable', body: 'See what works and double down on it.' },
    { icon: 'users', title: 'Built for teams', body: 'Everyone works from the same source of truth.' },
  ]
    .slice(0, n)
    .map((props) => ({ block: 'item/feature', props }));

export const SECTIONS: readonly SectionDef[] = [
  {
    block: 'nav/bar',
    family: 'nav',
    label: 'Navigation bar',
    description: 'Logo, links and one button',
    fields: [f('logo', 'Logo text'), f('cta_label', 'Button', 'text', { half: true }), f('cta_href', 'Button link', 'url', { half: true })],
    item: 'item/link',
    keywords: 'nav header menu logo',
    props: { logo: 'acme', cta_label: 'Get started', cta_href: '#' },
    style: { spacing: 's' },
    blocks: [
      { block: 'item/link', props: { label: 'Features', href: '#features' } },
      { block: 'item/link', props: { label: 'Pricing', href: '#pricing' } },
      { block: 'item/link', props: { label: 'FAQ', href: '#faq' } },
    ],
  },
  {
    block: 'hero/split',
    family: 'hero',
    label: 'Hero · split with media',
    description: 'Headline left, media right',
    fields: HERO_FIELDS,
    media: ['embed/3d', 'embed/image'],
    keywords: 'hero header above the fold split 3d',
    props: { ...HERO_PROPS, trust: '★★★★★ 4.8 from 12,000 customers' },
    style: { background: 'gradient:soft', spacing: 'l', align: 'left', motion: 'fade-up' },
    blocks: [{ block: 'embed/3d', props: { src: '', alt: 'A product rendered in 3D', caption: '', auto_rotate: true, tilt_on_hover: true } }],
  },
  {
    block: 'hero/centered',
    family: 'hero',
    label: 'Hero · centered',
    description: 'Big centered statement',
    fields: HERO_FIELDS,
    media: ['embed/image', 'embed/3d'],
    keywords: 'hero centered statement launch',
    props: HERO_PROPS,
    style: { background: 'gradient:soft', spacing: 'xl', align: 'center', motion: 'fade-up' },
    blocks: [],
  },
  {
    block: 'hero/fullbleed',
    family: 'hero',
    label: 'Hero · full bleed',
    description: 'Dark, dramatic, edge to edge',
    fields: HERO_FIELDS,
    media: ['embed/3d', 'embed/image'],
    keywords: 'hero dark dramatic fullscreen',
    props: HERO_PROPS,
    style: { background: 'gradient:ink', spacing: 'xl', align: 'center', motion: 'parallax', min_height: '80vh' },
    blocks: [],
  },
  {
    block: 'hero/stacked',
    family: 'hero',
    label: 'Hero · stacked',
    description: 'Headline over a wide visual',
    fields: HERO_FIELDS,
    media: ['embed/image', 'embed/3d'],
    keywords: 'hero stacked screenshot product image',
    props: HERO_PROPS,
    style: { background: 'token:color.surface', spacing: 'l', align: 'left', motion: 'fade-up' },
    blocks: [{ block: 'embed/image', props: { src: '', alt: 'Product screenshot', caption: '' } }],
  },
  {
    block: 'features/grid',
    family: 'features',
    label: 'Features · 3-up grid',
    description: 'Icon cards in a grid',
    fields: HEADING_FIELDS,
    item: 'item/feature',
    keywords: 'features benefits cards grid icons',
    props: { eyebrow: 'Why us', heading: 'Everything you need, nothing you don’t.', subheading: '' },
    style: { spacing: 'l', motion: 'scroll-reveal' },
    blocks: features(3),
  },
  {
    block: 'features/alternating',
    family: 'features',
    label: 'Features · alternating rows',
    description: 'Image and text, side by side',
    fields: HEADING_FIELDS,
    item: 'item/feature',
    keywords: 'features rows zigzag image text',
    props: { eyebrow: 'How it helps', heading: 'Built around the way you work.', subheading: '' },
    style: { spacing: 'l', motion: 'fade-up' },
    blocks: features(3),
  },
  {
    block: 'features/bento',
    family: 'features',
    label: 'Features · bento',
    description: 'A lively mosaic of cards',
    fields: HEADING_FIELDS,
    item: 'item/feature',
    keywords: 'features bento mosaic cards',
    props: { eyebrow: 'Highlights', heading: 'Small touches, big difference.', subheading: '' },
    style: { background: 'token:color.surface-alt', spacing: 'l', motion: 'tilt-hover' },
    blocks: features(5),
  },
  {
    block: 'features/steps',
    family: 'features',
    label: 'How it works · steps',
    description: 'Numbered steps with a connector',
    fields: HEADING_FIELDS,
    item: 'item/step',
    keywords: 'how it works steps process numbered',
    props: { eyebrow: 'How it works', heading: 'Up and running in three steps.', subheading: '' },
    style: { spacing: 'l', motion: 'scroll-reveal' },
    blocks: [
      { block: 'item/step', props: { title: 'Sign up', body: 'Create your account in under a minute.' } },
      { block: 'item/step', props: { title: 'Make it yours', body: 'Pick a plan and set your preferences.' } },
      { block: 'item/step', props: { title: 'Enjoy', body: 'Start seeing results from day one.' } },
    ],
  },
  {
    block: 'features/stats',
    family: 'features',
    label: 'Stats · big numbers',
    description: 'Proof in three numbers',
    fields: HEADING_FIELDS,
    item: 'item/stat',
    keywords: 'stats numbers metrics proof',
    props: { eyebrow: '', heading: 'The numbers speak.', subheading: '' },
    style: { background: 'gradient:primary', spacing: 'm', align: 'center', motion: 'scroll-reveal' },
    blocks: [
      { block: 'item/stat', props: { value: '12k', label: 'happy customers' } },
      { block: 'item/stat', props: { value: '4.8★', label: 'average rating' } },
      { block: 'item/stat', props: { value: '2×', label: 'faster than before' } },
    ],
  },
  {
    block: 'social/logos',
    family: 'social',
    label: 'Logo strip',
    description: 'Trusted-by wordmarks',
    fields: [f('label', 'Label')],
    item: 'item/logo',
    keywords: 'logos trusted by customers partners',
    props: { label: 'Trusted by teams at' },
    style: { spacing: 's', align: 'center' },
    blocks: ['Northwind', 'Fabrikam', 'Contoso', 'Lumen&Co', 'Tailspin'].map((name) => ({ block: 'item/logo', props: { name } })),
  },
  {
    block: 'social/testimonials',
    family: 'social',
    label: 'Testimonials',
    description: 'Quote cards with avatars',
    fields: HEADING_FIELDS,
    item: 'item/testimonial',
    keywords: 'testimonials reviews quotes customers',
    props: { eyebrow: 'Loved by customers', heading: 'Don’t take our word for it.', subheading: '' },
    style: { background: 'token:color.surface-alt', spacing: 'l', motion: 'scroll-reveal' },
    blocks: [
      { block: 'item/testimonial', props: { quote: 'The easiest decision we made this year. The team adopted it in a day.', name: 'Priya Natarajan', role: 'Head of Ops, Fabrikam' } },
      { block: 'item/testimonial', props: { quote: 'It feels like it was built by people who actually use it.', name: 'Marco Bianchi', role: 'Founder, Lumen&Co' } },
    ],
  },
  {
    block: 'social/quote',
    family: 'social',
    label: 'Big quote',
    description: 'One testimonial, center stage',
    fields: [f('quote', 'Quote', 'textarea'), f('name', 'Name', 'text', { half: true }), f('role', 'Role', 'text', { half: true })],
    keywords: 'quote testimonial single review',
    props: { quote: 'We shipped our launch page in an afternoon — and it converted better than anything we’d built by hand.', name: 'Dana Levi', role: 'Product lead, Acme' },
    style: { spacing: 'l', align: 'center', motion: 'fade-up' },
    blocks: [],
  },
  {
    block: 'pricing/tiers',
    family: 'pricing',
    label: 'Pricing · tier cards',
    description: 'Three plans, one highlighted',
    fields: HEADING_FIELDS,
    item: 'item/tier',
    keywords: 'pricing plans tiers cards',
    props: { eyebrow: 'Pricing', heading: 'Simple plans that grow with you.', subheading: 'Start free. Upgrade when you are ready.' },
    style: { spacing: 'l', align: 'center', motion: 'tilt-hover' },
    blocks: [
      { block: 'item/tier', props: { name: 'Free', price: '$0', period: '/month', blurb: 'Everything to get started.', features: ['Up to 3 projects', 'Community support'], cta_label: 'Start free', cta_href: '#', highlight: false, badge: '' } },
      { block: 'item/tier', props: { name: 'Plus', price: '$12', period: '/month', blurb: 'For people who want more.', features: ['Unlimited projects', 'Priority support', 'Custom domain'], cta_label: 'Choose Plus', cta_href: '#', highlight: true, badge: 'Most popular' } },
      { block: 'item/tier', props: { name: 'Team', price: '$29', period: '/seat', blurb: 'Collaborate at scale.', features: ['Everything in Plus', 'Shared workspaces', 'SSO'], cta_label: 'Talk to us', cta_href: '#', highlight: false, badge: '' } },
    ],
  },
  {
    block: 'pricing/compare',
    family: 'pricing',
    label: 'Pricing · comparison table',
    description: 'Plans side by side, perk by perk',
    fields: HEADING_FIELDS,
    item: 'item/tier',
    keywords: 'pricing compare table matrix',
    props: { eyebrow: '', heading: 'Compare plans', subheading: '' },
    style: { spacing: 'l' },
    blocks: [
      { block: 'item/tier', props: { name: 'Free', price: '$0', period: '', blurb: '', features: ['Projects', 'Community support'], cta_label: 'Start free', cta_href: '#', highlight: false, badge: '' } },
      { block: 'item/tier', props: { name: 'Plus', price: '$12', period: '/mo', blurb: '', features: ['Projects', 'Community support', 'Priority support', 'Custom domain'], cta_label: 'Choose Plus', cta_href: '#', highlight: true, badge: '' } },
    ],
  },
  {
    block: 'pricing/single',
    family: 'pricing',
    label: 'Pricing · single plan',
    description: 'One price, all included',
    fields: [
      f('heading', 'Heading', 'textarea'),
      f('subheading', 'Subheading', 'textarea'),
      f('name', 'Plan', 'text', { half: true }),
      f('price', 'Price', 'text', { half: true }),
      f('period', 'Period', 'text', { half: true }),
      f('badge', 'Badge', 'text', { half: true }),
      f('features', 'Included (one per line)', 'lines'),
      f('cta_label', 'Button', 'text', { half: true }),
      f('cta_href', 'Button link', 'url', { half: true }),
    ],
    keywords: 'pricing single plan price',
    props: { heading: 'One plan. Everything included.', subheading: 'No tiers to decode.', name: 'All access', price: '$9', period: '/month', badge: 'Cancel anytime', features: ['Every feature', 'Unlimited use', 'Human support'], cta_label: 'Get all access', cta_href: '#' },
    style: { background: 'token:color.surface-alt', spacing: 'l', align: 'center', motion: 'fade-up' },
    blocks: [],
  },
  {
    block: 'faq/accordion',
    family: 'faq',
    label: 'FAQ · accordion',
    description: 'Questions that expand',
    fields: HEADING_FIELDS,
    item: 'item/faq',
    keywords: 'faq questions accordion answers',
    props: { eyebrow: '', heading: 'Questions, answered', subheading: 'Still curious? Our team answers in minutes.' },
    style: { spacing: 'l' },
    blocks: [
      { block: 'item/faq', props: { question: 'Is there a free plan?', answer: 'Yes. Start free and upgrade whenever you like — no card needed.' } },
      { block: 'item/faq', props: { question: 'Can I cancel anytime?', answer: 'Of course. You keep access until the end of your billing period.' } },
      { block: 'item/faq', props: { question: 'Do you offer support?', answer: 'Real people, every day, usually within the hour.' } },
    ],
  },
  {
    block: 'faq/grid',
    family: 'faq',
    label: 'FAQ · two-column grid',
    description: 'All answers visible',
    fields: HEADING_FIELDS,
    item: 'item/faq',
    keywords: 'faq grid questions',
    props: { eyebrow: 'FAQ', heading: 'Good questions', subheading: '' },
    style: { spacing: 'l' },
    blocks: [
      { block: 'item/faq', props: { question: 'How long does setup take?', answer: 'Most people are done in five minutes.' } },
      { block: 'item/faq', props: { question: 'Where is my data stored?', answer: 'In the region you choose, encrypted at rest.' } },
      { block: 'item/faq', props: { question: 'Can I invite my team?', answer: 'Yes — invite as many people as your plan allows.' } },
      { block: 'item/faq', props: { question: 'Is there an API?', answer: 'A full REST API, documented and versioned.' } },
    ],
  },
  {
    block: 'cta/band',
    family: 'cta',
    label: 'CTA · band',
    description: 'A bold full-width call to action',
    fields: [
      f('headline', 'Headline', 'textarea'),
      f('subhead', 'Subhead'),
      f('primary_label', 'Button', 'text', { half: true }),
      f('primary_href', 'Button link', 'url', { half: true }),
      f('secondary_label', 'Secondary button', 'text', { half: true }),
      f('secondary_href', 'Secondary link', 'url', { half: true }),
    ],
    keywords: 'cta call to action band banner signup',
    props: { headline: 'Start today.', subhead: 'Join free — it takes a minute.', primary_label: 'Get started free', primary_href: '#', secondary_label: '', secondary_href: '' },
    style: { background: 'token:color.primary', spacing: 'l', align: 'center', motion: 'fade-up' },
    blocks: [],
  },
  {
    block: 'cta/split',
    family: 'cta',
    label: 'CTA · email capture',
    description: 'Headline and an email field',
    fields: [
      f('headline', 'Headline', 'textarea'),
      f('subhead', 'Subhead', 'textarea'),
      f('placeholder', 'Field placeholder', 'text', { half: true }),
      f('button_label', 'Button', 'text', { half: true }),
      f('form_action', 'Form action (https)', 'url', { hint: 'Where the form posts, e.g. your newsletter provider. Empty = no submission.' }),
      f('note', 'Small print'),
    ],
    keywords: 'cta email capture waitlist newsletter form',
    props: { headline: 'Be the first to know.', subhead: 'One email when we launch. No spam, ever.', placeholder: 'you@company.com', button_label: 'Join the waitlist', form_action: '', note: 'Unsubscribe anytime.' },
    style: { background: 'gradient:ink', spacing: 'l', align: 'left', motion: 'fade-up' },
    blocks: [],
  },
  {
    block: 'cta/card',
    family: 'cta',
    label: 'CTA · floating card',
    description: 'A card that lifts off the page',
    fields: [
      f('eyebrow', 'Eyebrow'),
      f('headline', 'Headline', 'textarea'),
      f('subhead', 'Subhead'),
      f('primary_label', 'Button', 'text', { half: true }),
      f('primary_href', 'Button link', 'url', { half: true }),
    ],
    keywords: 'cta card callout',
    props: { eyebrow: 'Ready?', headline: 'Let’s build something great.', subhead: 'Talk to us — we reply the same day.', primary_label: 'Book a call', primary_href: '#' },
    style: { spacing: 'l', align: 'center', motion: 'tilt-hover' },
    blocks: [],
  },
  {
    block: 'content/text',
    family: 'content',
    label: 'Text',
    description: 'A heading and a few paragraphs',
    fields: [f('eyebrow', 'Eyebrow'), f('heading', 'Heading', 'textarea'), f('body', 'Body (blank line = new paragraph)', 'textarea')],
    keywords: 'text about story paragraph content',
    props: { eyebrow: 'Our story', heading: 'Why we built this', body: 'We were tired of tools that got in the way.\n\nSo we built one that gets out of it.' },
    style: { spacing: 'l', motion: 'fade-up' },
    blocks: [],
  },
  {
    block: 'media/3d-embed',
    family: 'media',
    label: '3D embed',
    description: 'A live 3D artifact from your library',
    fields: HEADING_FIELDS,
    media: ['embed/3d'],
    keywords: '3d model embed scene product turntable',
    props: { eyebrow: 'Take a closer look', heading: 'Turn it around.', subheading: '' },
    style: { background: 'gradient:soft', spacing: 'l', align: 'center', motion: 'fade-up' },
    blocks: [{ block: 'embed/3d', props: { src: '', alt: 'Interactive 3D view', caption: '', auto_rotate: true, tilt_on_hover: true } }],
  },
  {
    block: 'media/video',
    family: 'media',
    label: 'Video',
    description: 'A framed video with a poster',
    fields: [
      f('heading', 'Heading', 'textarea'),
      f('subheading', 'Subheading'),
      f('src', 'Video URL (mp4/webm, https)', 'url'),
      f('poster', 'Poster image', 'media'),
      f('caption', 'Caption'),
    ],
    keywords: 'video demo film play',
    props: { heading: 'See it in 60 seconds.', subheading: '', src: '', poster: '', caption: '' },
    style: { spacing: 'l', align: 'center', motion: 'fade-up' },
    blocks: [],
  },
  {
    block: 'media/gallery',
    family: 'media',
    label: 'Gallery',
    description: 'A grid of images',
    fields: HEADING_FIELDS,
    item: 'embed/image',
    keywords: 'gallery images photos portfolio grid',
    props: { eyebrow: '', heading: 'Selected work', subheading: '' },
    style: { spacing: 'l', motion: 'scroll-reveal' },
    blocks: [1, 2, 3, 4, 5, 6].map((n) => ({ block: 'embed/image', props: { src: '', alt: `Project ${n}`, caption: `Project ${n}` } })),
  },
  {
    block: 'footer/columns',
    family: 'footer',
    label: 'Footer · columns',
    description: 'Logo, tagline and link columns',
    fields: [f('logo', 'Logo text'), f('tagline', 'Tagline', 'textarea'), f('legal', 'Legal line')],
    item: 'item/column',
    keywords: 'footer links columns legal',
    props: { logo: 'acme', tagline: 'Made with care for people who ship.', legal: '© 2026 Acme Inc.' },
    style: { background: 'token:color.ink', spacing: 'm' },
    blocks: [
      { block: 'item/column', props: { title: 'Product', links: [{ label: 'Features', href: '#features' }, { label: 'Pricing', href: '#pricing' }] } },
      { block: 'item/column', props: { title: 'Company', links: [{ label: 'About', href: '#' }, { label: 'Careers', href: '#' }] } },
      { block: 'item/column', props: { title: 'Help', links: [{ label: 'Contact', href: 'mailto:hello@example.com' }, { label: 'Privacy', href: '#' }] } },
    ],
  },
  {
    block: 'footer/simple',
    family: 'footer',
    label: 'Footer · simple',
    description: 'One line: logo, links, legal',
    fields: [f('logo', 'Logo text'), f('links', 'Links', 'links'), f('legal', 'Legal line')],
    keywords: 'footer simple minimal',
    props: { logo: 'acme', legal: '© 2026 Acme Inc.', links: [{ label: 'Privacy', href: '#' }, { label: 'Terms', href: '#' }, { label: 'Contact', href: 'mailto:hello@example.com' }] },
    style: { spacing: 's' },
    blocks: [],
  },
];

const SECTION_BY_BLOCK = new Map(SECTIONS.map((s) => [s.block, s]));
const ITEM_BY_BLOCK = new Map(ITEMS.map((i) => [i.block, i]));

export const SECTION_BLOCKS: readonly string[] = SECTIONS.map((s) => s.block);
export const ITEM_BLOCKS: readonly string[] = ITEMS.map((i) => i.block);

export function sectionDef(block: string): SectionDef | undefined {
  return SECTION_BY_BLOCK.get(block);
}
export function itemDef(block: string): ItemDef | undefined {
  return ITEM_BY_BLOCK.get(block);
}
export function familyOf(block: string): Family | null {
  return sectionDef(block)?.family ?? null;
}
/** The other layouts a section can swap to (same family). */
export function layoutsOf(block: string): SectionDef[] {
  const fam = familyOf(block);
  return fam ? SECTIONS.filter((s) => s.family === fam) : [];
}
/** "Hero", "Features"… — the Layers name when a section has none. */
export function sectionLabel(s: Pick<SiteSection, 'block' | 'name'>): string {
  if (s.name?.trim()) return s.name.trim();
  const d = sectionDef(s.block);
  if (!d) return s.block;
  return d.label.split(' · ')[0];
}

/** Search the library (label, description, keywords, family). */
export function searchBlocks(q: string): SectionDef[] {
  const terms = q.toLowerCase().split(/\s+/).filter(Boolean);
  if (!terms.length) return [...SECTIONS];
  return SECTIONS.filter((s) => {
    const hay = `${s.label} ${s.description} ${s.keywords} ${s.family}`.toLowerCase();
    return terms.every((t) => hay.includes(t));
  });
}
