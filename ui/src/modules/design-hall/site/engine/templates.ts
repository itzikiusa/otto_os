// Site Studio — the six starter templates offered when a site is created
// (landing, product launch, event, portfolio, docs home, waitlist). Real copy,
// never lorem ipsum; every section uses a motion preset and brand tokens only,
// so a template re-skins itself the moment a brand kit is linked.

import { sectionDef } from './catalog';
import { clone } from './ops';
import type { SiteBlock, SiteDoc, SitePage, SiteProps, SiteSection, SiteStyle } from './types';

export interface SiteTemplate {
  id: string;
  name: string;
  description: string;
  build: (title: string) => SiteDoc;
}

type Items = [string, SiteProps][];

/** A section from the library with overrides (ids stay readable: `hero`, `faq`…). */
function sec(id: string, block: string, props: SiteProps = {}, opts: { style?: SiteStyle; items?: Items; name?: string } = {}): SiteSection {
  const def = sectionDef(block);
  if (!def) throw new Error(`template uses unknown block ${block}`);
  const blocks: SiteBlock[] = (opts.items ?? def.blocks.map((b) => [b.block, b.props] as [string, SiteProps])).map(([b, p], i) => ({
    id: `${id}-${i + 1}`,
    block: b,
    props: clone(p),
  }));
  const s: SiteSection = { id, block, props: { ...clone(def.props), ...props }, style: { ...clone(def.style), ...(opts.style ?? {}) }, blocks };
  if (opts.name) s.name = opts.name;
  return s;
}

function page(id: string, title: string, slug: string, sections: SiteSection[]): SitePage {
  return { id, title, slug, sections };
}

function site(title: string, domain: string, description: string, pages: SitePage[]): SiteDoc {
  return { type: 'otto-site', version: 1, title, settings: { domain, lang: 'en', description }, pages };
}

const link = (label: string, href: string): [string, SiteProps] => ['item/link', { label, href }];
const feat = (icon: string, title: string, body: string): [string, SiteProps] => ['item/feature', { icon, title, body }];
const step = (title: string, body: string): [string, SiteProps] => ['item/step', { title, body }];
const stat = (value: string, label: string): [string, SiteProps] => ['item/stat', { value, label }];
const faq = (question: string, answer: string): [string, SiteProps] => ['item/faq', { question, answer }];
const quote = (q: string, name: string, role: string): [string, SiteProps] => ['item/testimonial', { quote: q, name, role }];
const tier = (name: string, price: string, period: string, blurb: string, features: string[], cta: string, highlight = false, badge = ''): [string, SiteProps] => [
  'item/tier',
  { name, price, period, blurb, features, cta_label: cta, cta_href: '#', highlight, badge },
];
const embed3d = (label: string, alt: string): [string, SiteProps] => ['embed/3d', { src: '', alt, label, caption: '', auto_rotate: true, tilt_on_hover: true }];

// ── Landing ──────────────────────────────────────────────────────────────────

function landing(title: string): SiteDoc {
  const name = title.trim() || 'Rewards+';
  return site(name, 'rewardsplus.acme.example', 'Earn points faster, unlock tiers in weeks and spend them anywhere.', [
    page('home', 'Home', '', [
      sec('nav', 'nav/bar', { logo: `acme ${name}`, cta_label: 'Join free', cta_href: '#join' }, { items: [link('How it works', '#how'), link('Tiers', '#tiers'), link('FAQ', '#faq')], name: 'Nav' }),
      sec(
        'hero',
        'hero/split',
        {
          eyebrow: `New · ${name}`,
          headline: 'Every purchase moves you up.',
          subhead: 'Earn points 2× faster, unlock tiers in weeks, not months, and spend them anywhere Acme is.',
          primary_label: 'Join free',
          primary_href: '#join',
          secondary_label: 'See the tiers',
          secondary_href: '#tiers',
          trust: '★★★★★ 4.8 from 12,000 members',
        },
        { items: [embed3d(`acme ${name}`, `A rotating ${name} card`)], style: { motion: 'tilt-hover' } },
      ),
      sec('logos', 'social/logos', { label: 'Spend points at' }, { name: 'Logos strip' }),
      sec(
        'features',
        'features/grid',
        { eyebrow: `Why ${name}`, heading: 'Rewards that keep up with you.' },
        {
          items: [
            feat('2×', '2× points on weekends', 'Every Saturday and Sunday, in store and online. No codes, no opt-in.'),
            feat('layers', 'Tier perks that stack', 'Gold keeps every Silver perk and adds free delivery and early access.'),
            feat('infinity', 'Points never expire', 'Stay a member and your balance stays yours, even between seasons.'),
          ],
        },
      ),
      sec(
        'how',
        'features/steps',
        { eyebrow: 'How it works', heading: 'From sign-up to first reward in a week.' },
        { items: [step('Join free', 'One tap with your Acme account — no card needed.'), step('Shop as usual', 'Every purchase earns points, double on weekends.'), step('Level up', 'Hit Silver in weeks and unlock perks that stack.')] },
      ),
      sec(
        'tiers',
        'pricing/tiers',
        { eyebrow: 'Tiers', heading: 'Climb faster. Keep what you earn.', subheading: 'Every tier is free — you unlock them by shopping.' },
        {
          items: [
            tier('Silver', '0', ' pts', 'Where everyone starts.', ['1 point per $1', 'Birthday bonus', 'Member-only prices'], 'Join free'),
            tier('Gold', '2,500', ' pts', 'Most members get here in 6 weeks.', ['1.5 points per $1', 'Free delivery', 'Early access to drops'], 'See Gold perks', true, 'Most popular'),
            tier('Platinum', '10,000', ' pts', 'For our most loyal shoppers.', ['2 points per $1', 'Priority support', 'Invite-only events'], 'See Platinum'),
          ],
          name: 'Tiers',
        },
      ),
      sec(
        'testimonials',
        'social/testimonials',
        { eyebrow: 'Members love it', heading: 'Small rewards, every day.' },
        {
          items: [
            quote('I hit Gold in five weeks without changing how I shop. Free delivery alone paid for itself.', 'Noa Peretz', 'Gold member'),
            quote('Weekend double points are my favourite thing. I plan my big shop around them now.', 'James Okafor', 'Silver member'),
          ],
        },
      ),
      sec(
        'faq',
        'faq/accordion',
        { heading: 'Questions, answered', subheading: 'Still curious? Our team answers in minutes.' },
        {
          items: [
            faq(`Is ${name} really free?`, 'Yes. Joining and every tier are free — you unlock tiers by shopping.'),
            faq('How fast can I reach Gold?', 'Most members reach Gold in about six weeks of regular shopping.'),
            faq('Where can I spend my points?', 'Anywhere Acme is: in store, online and in the app.'),
            faq('Do points expire?', 'Never, as long as you stay a member.'),
          ],
        },
      ),
      sec('join', 'cta/band', { headline: 'Start earning today.', subhead: 'Join free and get 200 points on us.', primary_label: `Join ${name} free`, primary_href: '#' }, { name: 'CTA band' }),
      sec('footer', 'footer/columns', { logo: `acme ${name}`, tagline: 'Earn on every purchase, spend anywhere Acme is.', legal: `© 2026 Acme Inc. ${name} terms apply.` }, {
        name: 'Footer',
        items: [
          ['item/column', { title: name, links: [{ label: 'How it works', href: '#how' }, { label: 'Tiers', href: '#tiers' }, { label: 'FAQ', href: '#faq' }] }],
          ['item/column', { title: 'Acme', links: [{ label: 'Stores', href: '#' }, { label: 'Careers', href: '#' }, { label: 'Press', href: '#' }] }],
          ['item/column', { title: 'Help', links: [{ label: 'Contact', href: 'mailto:help@acme.example' }, { label: 'Terms', href: '#' }, { label: 'Privacy', href: '#' }] }],
        ],
      }),
    ]),
  ]);
}

// ── Product launch ───────────────────────────────────────────────────────────

function launch(title: string): SiteDoc {
  const name = title.trim() || 'Orbit One';
  return site(name, 'orbit.example', `${name} — sound that follows you.`, [
    page('home', 'Home', '', [
      sec('nav', 'nav/bar', { logo: name, cta_label: 'Pre-order', cta_href: '#preorder' }, { items: [link('Design', '#design'), link('Specs', '#specs'), link('FAQ', '#faq')] }),
      sec(
        'hero',
        'hero/fullbleed',
        {
          eyebrow: 'Available for pre-order',
          headline: `Meet ${name}.\nSound that follows you.`,
          subhead: 'Spatial audio that tracks your head, 40-hour battery and a case that fits in a coin pocket.',
          primary_label: 'Pre-order — $249',
          primary_href: '#preorder',
          secondary_label: 'Watch the film',
          secondary_href: '#film',
          trust: 'Ships October 14 · Free returns for 60 days',
        },
        { items: [embed3d(name, `${name} headphones, rotating`)], style: { min_height: '100vh' } },
      ),
      sec(
        'design',
        'features/bento',
        { eyebrow: 'Designed around you', heading: 'Every detail, reconsidered.' },
        {
          items: [
            feat('sparkle', 'Adaptive spatial audio', 'Sound stays anchored to the scene as you turn your head.'),
            feat('clock', '40-hour battery', 'A week of commutes on one charge.'),
            feat('shield', 'Quiet mode', 'Noise cancelling that adapts every 1/200th of a second.'),
            feat('heart', '38 grams', 'So light you forget you are wearing them.'),
            feat('globe', 'Seamless switching', 'Laptop to phone without a tap.'),
          ],
        },
      ),
      sec('specs-3d', 'media/3d-embed', { eyebrow: 'Take a closer look', heading: 'Turn it around. Every angle, every finish.' }, { items: [embed3d(name, `Interactive 3D view of ${name}`)], name: '3D showcase' }),
      sec('specs', 'features/stats', { heading: 'Built to go the distance.' }, { items: [stat('40h', 'battery life'), stat('38g', 'per earbud'), stat('IPX5', 'sweat and rain proof')] }),
      sec('preorder', 'pricing/single', {
        heading: 'Reserve yours today.',
        subheading: 'Pay nothing until it ships. Cancel anytime before then.',
        name: name,
        price: '$249',
        period: '',
        badge: 'Ships Oct 14',
        features: ['Charging case + USB-C cable', 'Three ear-tip sizes', '2-year warranty', 'Free engraving'],
        cta_label: 'Pre-order now',
        cta_href: '#',
      }),
      sec(
        'faq',
        'faq/grid',
        { eyebrow: 'FAQ', heading: 'Before you order' },
        {
          items: [
            faq('When will it ship?', 'Pre-orders ship from October 14, in the order they were placed.'),
            faq('Does it work with my phone?', 'Yes — iOS and Android, plus any Bluetooth 5.3 device.'),
            faq('Can I cancel my pre-order?', 'Anytime before it ships, with a full refund.'),
            faq('What colours are there?', 'Graphite, Chalk and a limited Sunset edition.'),
          ],
        },
      ),
      sec('cta', 'cta/card', { eyebrow: 'Limited first run', headline: 'Be first to hear it.', subhead: 'The launch edition comes with free engraving.', primary_label: 'Pre-order now', primary_href: '#preorder' }),
      sec('footer', 'footer/simple', { logo: name, legal: `© 2026 ${name} Audio.` }),
    ]),
  ]);
}

// ── Event ────────────────────────────────────────────────────────────────────

function event(title: string): SiteDoc {
  const name = title.trim() || 'Frontier Summit 2026';
  return site(name, 'frontier-summit.example', `${name} — two days on what's next in product design. Lisbon, October 14–15.`, [
    page('home', 'Home', '', [
      sec('nav', 'nav/bar', { logo: name, cta_label: 'Get tickets', cta_href: '#tickets' }, { items: [link('Agenda', '#agenda'), link('Tickets', '#tickets'), link('FAQ', '#faq')] }),
      sec(
        'hero',
        'hero/centered',
        {
          eyebrow: 'Lisbon · October 14–15, 2026',
          headline: 'Two days on\nwhat ships next.',
          subhead: '40 speakers, 12 workshops and 900 of the people building the next decade of products — on the river in Lisbon.',
          primary_label: 'Get your ticket',
          primary_href: '#tickets',
          secondary_label: 'See the agenda',
          secondary_href: '#agenda',
          trust: 'Early-bird prices end September 30',
        },
        { style: { background: 'gradient:sunset', motion: 'parallax' } },
      ),
      sec('numbers', 'features/stats', { heading: '' }, { items: [stat('40', 'speakers'), stat('12', 'hands-on workshops'), stat('900', 'attendees')], style: { background: 'token:color.ink' } }),
      sec(
        'agenda',
        'features/steps',
        { eyebrow: 'Agenda', heading: 'Two days, one arc.' },
        {
          items: [
            step('Day 1 · Morning', 'Keynotes: the state of product, and where AI actually helps.'),
            step('Day 1 · Afternoon', 'Workshops in small groups, led by the people who built it.'),
            step('Day 2 · All day', 'Case studies, live critiques and the closing party on the river.'),
          ],
        },
      ),
      sec(
        'voices',
        'social/testimonials',
        { eyebrow: 'Last year', heading: 'Why people come back.' },
        {
          items: [
            quote('The only conference where every talk changed how I work on Monday.', 'Ines Duarte', 'Design lead, Tailspin'),
            quote('I hired two people and found a co-founder. In two days.', 'Tomás Reyes', 'Founder, Contoso Labs'),
          ],
        },
      ),
      sec(
        'tickets',
        'pricing/tiers',
        { eyebrow: 'Tickets', heading: 'Pick your pass.', subheading: 'Prices include VAT, lunch both days and the closing party.' },
        {
          items: [
            tier('Online', '€49', '', 'Every keynote, live and on demand.', ['Live stream', 'Recordings for a year', 'Community chat'], 'Watch online'),
            tier('Conference', '€590', '', 'The full two days in Lisbon.', ['All talks and workshops', 'Lunch both days', 'Closing party'], 'Get Conference', true, 'Early bird'),
            tier('Team of 5', '€2,400', '', 'Bring the whole squad.', ['Five conference passes', 'Reserved seating', 'Team dinner'], 'Book for your team'),
          ],
        },
      ),
      sec(
        'faq',
        'faq/accordion',
        { heading: 'Good to know' },
        {
          items: [
            faq('Where exactly is it?', 'At the Tejo Hall, a ten-minute walk from Cais do Sodré station.'),
            faq('Can I get a refund?', 'Full refunds until September 30; transfers to a colleague anytime.'),
            faq('Is it accessible?', 'The venue is step-free, with live captions on every main-stage talk.'),
          ],
        },
      ),
      sec('cta', 'cta/band', { headline: 'See you in Lisbon.', subhead: 'Early-bird prices end September 30.', primary_label: 'Get your ticket', primary_href: '#tickets' }, { style: { background: 'gradient:primary' } }),
      sec('footer', 'footer/simple', { logo: name, legal: '© 2026 Frontier Events Ltd.' }),
    ]),
  ]);
}

// ── Portfolio ────────────────────────────────────────────────────────────────

function portfolio(title: string): SiteDoc {
  const name = title.trim() || 'Maya Chen';
  return site(name, 'mayachen.example', `${name} — product designer. Calm interfaces for complicated things.`, [
    page('home', 'Work', '', [
      sec('nav', 'nav/bar', { logo: name, cta_label: 'Get in touch', cta_href: 'mailto:hello@mayachen.example' }, { items: [link('Work', '#work'), link('About', '#about'), link('Contact', '#contact')] }),
      sec(
        'hero',
        'hero/centered',
        {
          eyebrow: 'Product designer · Tel Aviv',
          headline: 'I design calm interfaces\nfor complicated things.',
          subhead: 'Ten years shaping fintech, health and developer tools — from the first sketch to the thousandth user.',
          primary_label: 'See selected work',
          primary_href: '#work',
          secondary_label: 'Read about me',
          secondary_href: '#about',
          trust: '',
        },
        { style: { background: 'token:color.surface', spacing: 'xl' } },
      ),
      sec('work', 'media/gallery', { eyebrow: 'Selected work', heading: 'Recent projects', subheading: '' }, {
        items: [
          ['embed/image', { src: '', alt: 'Ledger — a banking app redesign', caption: 'Ledger · Banking app, 2026' }],
          ['embed/image', { src: '', alt: 'Pulse — patient dashboard', caption: 'Pulse · Health dashboard, 2025' }],
          ['embed/image', { src: '', alt: 'Relay — developer console', caption: 'Relay · Developer console, 2025' }],
          ['embed/image', { src: '', alt: 'Harbor — design system', caption: 'Harbor · Design system, 2024' }],
          ['embed/image', { src: '', alt: 'Atlas — travel planner', caption: 'Atlas · Travel planner, 2024' }],
          ['embed/image', { src: '', alt: 'Loom — onboarding flow', caption: 'Loom · Onboarding, 2023' }],
        ],
      }),
      sec('about', 'content/text', {
        eyebrow: 'About',
        heading: 'Clarity is a feature.',
        body: 'I help teams turn complicated products into ones people trust — by listening first, prototyping early and sweating the last 10%.\n\nBefore going independent I led design at two fintech startups and built the design system at a health-tech scale-up.',
      }),
      sec('quote', 'social/quote', { quote: 'Maya found the simple product hiding inside our complicated one. Activation went up 38% in a quarter.', name: 'Daniel Katz', role: 'CEO, Ledger' }, { style: { background: 'token:color.surface-alt' } }),
      sec('contact', 'cta/card', { eyebrow: 'Available from November', headline: 'Let’s make something clear.', subhead: 'I take on two projects at a time.', primary_label: 'hello@mayachen.example', primary_href: 'mailto:hello@mayachen.example' }),
      sec('footer', 'footer/simple', { logo: name, legal: `© 2026 ${name}`, links: [{ label: 'LinkedIn', href: 'https://www.linkedin.com' }, { label: 'Dribbble', href: 'https://dribbble.com' }, { label: 'Email', href: 'mailto:hello@mayachen.example' }] }),
    ]),
  ]);
}

// ── Docs home ────────────────────────────────────────────────────────────────

function docs(title: string): SiteDoc {
  const name = title.trim() || 'Relay Docs';
  return site(name, 'docs.relay.example', `${name} — guides, API reference and SDKs.`, [
    page('home', 'Home', '', [
      sec('nav', 'nav/bar', { logo: name, cta_label: 'Get an API key', cta_href: '#quickstart' }, { items: [link('Guides', '#guides'), link('API', '#guides'), link('Changelog', '#updates')] }),
      sec(
        'hero',
        'hero/centered',
        {
          eyebrow: 'Documentation',
          headline: 'Build with Relay in minutes.',
          subhead: 'Guides, API reference and SDKs for sending messages at any scale.',
          primary_label: 'Start the quickstart',
          primary_href: '#quickstart',
          secondary_label: 'Browse the API',
          secondary_href: '#guides',
          trust: 'v4.2 · Updated September 2026',
        },
        { style: { spacing: 'l' } },
      ),
      sec(
        'guides',
        'features/grid',
        { eyebrow: 'Start here', heading: 'Find your way around.' },
        {
          items: [
            feat('rocket', 'Quickstart', 'Send your first message in five minutes with any language.'),
            feat('book', 'Guides', 'Webhooks, retries, templates and everything in between.'),
            feat('code', 'API reference', 'Every endpoint, parameter and error, with examples.'),
            feat('layers', 'SDKs', 'Official libraries for TypeScript, Python, Go and Rust.'),
            feat('shield', 'Security', 'Auth, signing secrets and how we keep data safe.'),
            feat('chart', 'Limits & pricing', 'Rate limits, quotas and how usage is billed.'),
          ],
        },
      ),
      sec(
        'quickstart',
        'features/steps',
        { eyebrow: 'Quickstart', heading: 'Your first message in three steps.' },
        { items: [step('Create a key', 'Generate an API key from your dashboard.'), step('Install the SDK', 'npm install @relay/sdk — or pip, go get, cargo add.'), step('Send it', 'Call relay.messages.send() and watch it land.')] },
      ),
      sec(
        'faq',
        'faq/grid',
        { eyebrow: 'FAQ', heading: 'Common questions' },
        {
          items: [
            faq('Is there a sandbox?', 'Every account has a free sandbox with test keys.'),
            faq('Which regions do you run in?', 'US, EU and APAC — pick per project.'),
            faq('How are webhooks signed?', 'HMAC-SHA256 with your signing secret; see the Security guide.'),
            faq('Where do I report a bug?', 'Open an issue on GitHub or email support.'),
          ],
        },
      ),
      sec('updates', 'cta/split', { headline: 'Never miss a breaking change.', subhead: 'One email per release with the changelog and migration notes.', placeholder: 'you@company.com', button_label: 'Subscribe', note: 'Monthly at most.' }),
      sec('footer', 'footer/columns', { logo: name, tagline: 'Messaging infrastructure for developers.', legal: '© 2026 Relay Inc.' }, {
        items: [
          ['item/column', { title: 'Docs', links: [{ label: 'Quickstart', href: '#quickstart' }, { label: 'Guides', href: '#guides' }, { label: 'API', href: '#guides' }] }],
          ['item/column', { title: 'Community', links: [{ label: 'GitHub', href: 'https://github.com' }, { label: 'Discord', href: '#' }] }],
          ['item/column', { title: 'Company', links: [{ label: 'Status', href: '#' }, { label: 'Contact', href: 'mailto:support@relay.example' }] }],
        ],
      }),
    ]),
  ]);
}

// ── Waitlist ─────────────────────────────────────────────────────────────────

function waitlist(title: string): SiteDoc {
  const name = title.trim() || 'Tandem';
  return site(name, 'tandem.example', `${name} — coming soon. Join the waitlist.`, [
    page('home', 'Home', '', [
      sec('nav', 'nav/bar', { logo: name, cta_label: 'Join the waitlist', cta_href: '#waitlist' }, { items: [] }),
      sec(
        'hero',
        'hero/fullbleed',
        {
          eyebrow: 'Coming this winter',
          headline: 'Plan trips together,\nwithout the group chat.',
          subhead: `${name} keeps everyone's dates, budgets and must-sees in one calm place — and finds the plan that works for all of you.`,
          primary_label: 'Join the waitlist',
          primary_href: '#waitlist',
          secondary_label: '',
          secondary_href: '',
          trust: '2,300 people are already waiting',
        },
        { style: { min_height: '80vh' } },
      ),
      sec(
        'teasers',
        'features/grid',
        { eyebrow: 'What’s coming', heading: 'Less back-and-forth. More actual trip.' },
        {
          items: [
            feat('calendar', 'Dates that work', 'Everyone marks their free days; we find the overlap.'),
            feat('users', 'One shared plan', 'Ideas, votes and bookings in one place — not five apps.'),
            feat('pin', 'Places you’ll love', 'Suggestions that fit the whole group’s taste and budget.'),
          ],
        },
      ),
      sec('quote', 'social/quote', { quote: 'I have organised six group trips. This is the first time I did not want to quit halfway.', name: 'Lior Ben-David', role: 'Beta tester' }),
      sec('waitlist', 'cta/split', { headline: 'Get early access.', subhead: 'We are letting people in a few hundred at a time. Leave your email and we will save you a spot.', placeholder: 'you@example.com', button_label: 'Join the waitlist', note: 'One email when it is your turn. That’s it.' }),
      sec('footer', 'footer/simple', { logo: name, legal: `© 2026 ${name}` }),
    ]),
  ]);
}

export const SITE_TEMPLATES: readonly SiteTemplate[] = [
  { id: 'landing', name: 'Landing page', description: 'Hero with a 3D card, features, tiers, FAQ and a CTA band', build: landing },
  { id: 'product-launch', name: 'Product launch', description: 'Dramatic dark hero, bento, 3D showcase, pre-order', build: launch },
  { id: 'event', name: 'Event', description: 'Date and place up top, agenda, tickets, FAQ', build: event },
  { id: 'portfolio', name: 'Portfolio', description: 'A calm intro, selected work, about and contact', build: portfolio },
  { id: 'docs-home', name: 'Docs home', description: 'Guides grid, quickstart steps, FAQ, changelog sign-up', build: docs },
  { id: 'waitlist', name: 'Waitlist', description: 'A teaser hero and an email capture that converts', build: waitlist },
];

export function templateById(id: string): SiteTemplate | undefined {
  return SITE_TEMPLATES.find((t) => t.id === id);
}

/** The document a new site starts as (a template, else an empty site). */
export function starterSite(templateId: string | undefined, title: string): SiteDoc {
  const t = templateId ? templateById(templateId) : undefined;
  if (t) {
    // Templates keep their sample brand (Rewards+, Orbit One…) in the copy — an
    // artifact title like "Autumn campaign site" is not a product name — and
    // take the artifact title as the site's name.
    const d = t.build('');
    if (title.trim()) d.title = title.trim();
    return d;
  }
  return { type: 'otto-site', version: 1, title: title.trim(), settings: { lang: 'en' }, pages: [] };
}
