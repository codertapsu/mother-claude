import { HelpTranslation } from '../help-localized';
import { VI_TOPICS_A } from './topics-a';
import { VI_TOPICS_B } from './topics-b';

/** The Help in Vietnamese (docs/I18N.md "Help"). */
export const VI_HELP: HelpTranslation = {
  groups: {
    basics: 'Cơ bản',
    sessions: 'Phiên của bạn',
    remote: 'Điện thoại & từ xa',
    data: 'Tài khoản & dữ liệu',
    'browser-teams': 'Trình duyệt & nhóm',
    advanced: 'Nâng cao',
    troubleshooting: 'Khắc phục sự cố',
  },
  topics: { ...VI_TOPICS_A, ...VI_TOPICS_B },
};
