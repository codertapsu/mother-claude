import { HelpTranslation } from '../help-localized';
import { VI_TOPICS_A } from './topics-a';
import { VI_TOPICS_B } from './topics-b';
import { VI_TOPICS_C } from './topics-c';

/** The Help in Vietnamese (docs/I18N.md "Help"). */
export const VI_HELP: HelpTranslation = {
  groups: {
    basics: 'Cơ bản',
    sessions: 'Phiên của bạn',
    remote: 'Điện thoại & từ xa',
    machines: 'Máy tính của bạn',
    data: 'Tài khoản & dữ liệu',
    'browser-teams': 'Trình duyệt & nhóm',
    advanced: 'Nâng cao',
    troubleshooting: 'Khắc phục sự cố',
  },
  topics: { ...VI_TOPICS_A, ...VI_TOPICS_B, ...VI_TOPICS_C },
};
