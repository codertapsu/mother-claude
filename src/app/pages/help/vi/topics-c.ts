import { HelpTopicSource } from '../help-model';

/*
 * Vietnamese Help (docs/I18N.md "Help"): your computers (Computers), and
 * their troubleshooting. Each topic has the same sections in the same order
 * as its English one in `help-topics.ts`; section ids, `where`, embeds and
 * link paths come from the English, only the link labels from here.
 * `**bold**` names a label as the Vietnamese interface shows it
 * (`i18n/vi/machines.json`, `moves.json`, `runs.json`); `…` stands for a
 * computer's or an account's name.
 */
const TOPICS: HelpTopicSource[] = [
  // ─────────────────────────────────────────────────── Máy tính của bạn
  {
    id: 'machines-setup',
    title: 'Thiết lập các máy tính của bạn',
    icon: 'monitor',
    group: 'machines',
    summary:
      'Kết nối các máy tính của bạn: các mục kiểm tra trên từng máy, cách khắc phục, bài kiểm tra trước khi bật, và những gì mỗi thẻ của trang Máy tính chứa.',
    intro:
      'Ghép nối các máy tính của bạn, rồi từ bất kỳ máy nào, bạn xem và điều khiển các phiên Claude Code của mọi máy, và chuyển một phiên, cùng mã nguồn của nó, sang máy tính khác rồi chuyển về. Mọi việc diễn ra trên trang **Máy tính**, vốn tắt cho đến khi bạn thiết lập nó trong ứng dụng máy tính. Việc thiết lập chạy trên từng máy, vì chỉ máy đó mới tự kiểm tra được chính nó.',
    keywords: [
      'máy tính',
      'máy tính khác',
      'máy thứ hai',
      'thiết lập',
      'kiểm tra',
      'tường lửa',
      'mạng cục bộ',
      'máy chủ chuyển tiếp',
      'tình trạng',
      'đường dẫn dài',
      'truy cập toàn bộ ổ đĩa',
      'dịch vụ nền',
    ],
    sections: [
      {
        heading: 'Bạn cần gì',
        points: [
          'Mother Claude, phiên bản này hoặc mới hơn, trên mỗi máy tính.',
          'Mỗi máy tính đều đang bật, có Internet hoặc ở cùng một mạng với các máy kia.',
          'Trên Windows, một lần đồng ý của quản trị viên, cho tường lửa.',
          'Khoảng 10 phút, khi có sẵn từng máy tính bên cạnh.',
        ],
      },
      {
        heading: 'Chạy thiết lập',
        where: 'desktop',
        intro:
          'Mở **Máy tính** và, trên thẻ **Đang tắt trên máy Mac này** (**Đang tắt trên máy PC này** trên Windows), nhấn **Thiết lập các máy tính của bạn**. Có bảy bước: **Tiếp** và **Quay lại** để chuyển giữa các bước, và **Tiếp tục thiết lập** trên thẻ đó đưa bạn về bước bạn đã dừng.',
        steps: [
          '**Máy Mac này** (hoặc **Máy PC này**): sửa những gì các mục kiểm tra phát hiện. Các mục bắt buộc được xếp trước.',
          '**Các máy tính của bạn**: ghép nối từng máy tính khác của bạn.',
          '**Mọi máy tính**: các mục kiểm tra của từng máy, đặt cạnh nhau.',
          '**Quyền của từng máy**: những gì mỗi máy được làm trên máy này. Nhấn **Lưu** sau khi thay đổi.',
          '**Tài khoản và thư mục**: tài khoản và thư mục dùng chung của bạn trên mọi máy.',
          '**Tùy chọn**: máy chủ chuyển tiếp, cách chuyển mã nguồn của phiên và nơi mã nguồn được đặt, cùng tùy chọn riêng của từng dự án.',
          '**Kiểm tra, rồi bật**: nhấn **Chạy kiểm tra**, rồi **Bật tính năng Máy tính**.',
        ],
        tips: [
          '**Bật tính năng Máy tính** bị làm mờ khi còn mục kiểm tra bắt buộc, và cho biết còn bao nhiêu mục. Cho đến khi bật, các máy tính của bạn chỉ ghép nối, chạy thiết lập và bài kiểm tra với nhau: không máy nào xem, điều khiển hay chuyển phiên của một máy còn đang tắt tính năng này.',
          'Sau đó các mục kiểm tra vẫn tiếp tục chạy: phần đầu trang **Máy tính** cho biết tình trạng các máy của bạn (**Mọi thứ đều ổn**, hoặc còn bao nhiêu mục khuyến nghị hay bắt buộc), thẻ của mỗi máy liệt kê các mục kiểm tra không đạt kèm cách sửa, và một chấm đỏ trên **Máy tính** ở thanh bên báo có gì đó lại hỏng.',
        ],
        links: [
          { label: 'Mở trang Máy tính', path: '/computers' },
          { label: 'Xem “Ghép nối các máy tính của bạn”', path: '/help/machines-pair' },
        ],
      },
      {
        heading: 'Những gì được kiểm tra',
        intro:
          'Mục **Bắt buộc** phải đạt thì tính năng mới bật được; mục **Khuyến nghị** vẫn hoạt động, nhưng kém hơn. Mỗi mục cho biết vì sao nó quan trọng, và nút của nó sẽ sửa hoặc mở đúng trang cài đặt.',
        terms: [
          {
            term: 'Mother Claude',
            text: 'Mỗi máy tính cần một phiên bản dùng cùng giao thức: máy dùng giao thức khác bắt buộc phải cập nhật. Máy chạy bản cũ hơn một máy khác được khuyến nghị cập nhật: **Cập nhật Mother Claude** mở **Cập nhật phần mềm**, còn trên máy khác thì **Nhờ … sửa** sẽ hiện việc đó ở máy ấy.',
          },
          {
            term: 'Luôn thức khi đang được dùng',
            text: 'Máy tính đang ngủ không trả lời được các máy khác. **Giữ máy thức khi đang được dùng** giữ máy thức khi có máy tính đã ghép nối đang kết nối; không cài đặt hệ thống nào bị thay đổi.',
          },
          {
            term: 'Khởi động khi bạn đăng nhập',
            text: 'Sau khi khởi động lại, các máy khác chỉ kết nối được với máy này khi Mother Claude chạy lại. **Bật** cho ứng dụng khởi động khi bạn đăng nhập, trong khay hệ thống, giống như **Cài đặt › Khởi động**.',
          },
          {
            term: 'Quyền truy cập Mạng cục bộ (máy Mac)',
            text: 'Không có quyền này, máy Mac chỉ tới được các máy tính khác của bạn qua máy chủ chuyển tiếp, kể cả khi ở nhà. Hãy cho phép khi macOS hỏi, hoặc nhấn **Mở cài đặt Mạng cục bộ** và bật Mother Claude.',
          },
          {
            term: 'Tường lửa (máy Mac)',
            text: 'Nếu tường lửa chặn Mother Claude, các máy khác chỉ tới được máy Mac qua máy chủ chuyển tiếp. Nhấn **Mở cài đặt Tường lửa** và cho phép Mother Claude.',
          },
          {
            term: 'Loại mạng (Windows)',
            text: 'Trên mạng Public, Windows chặn các kết nối đến. Nhấn **Mở cài đặt mạng** và chuyển mạng ở nhà hoặc văn phòng của bạn sang Private.',
          },
          {
            term: 'Tường lửa (Windows)',
            text: 'Không có quy tắc, các máy khác chỉ tới được máy PC qua máy chủ chuyển tiếp. **Cho phép qua tường lửa** thêm một quy tắc cho Mother Claude, sau khi Windows hỏi quản trị viên đồng ý.',
          },
          {
            term: 'Máy chủ chuyển tiếp',
            text: 'Khi khác mạng, các máy tính của bạn gặp nhau qua máy chủ chuyển tiếp. Hãy kiểm tra địa chỉ của nó, hoặc chọn máy chủ chuyển tiếp công cộng trong **Tùy chọn**.',
          },
          {
            term: 'Máy tính đã ghép nối',
            text: 'Chỉ những máy tính bạn đã ghép nối mới làm việc cùng nhau. Mỗi cặp máy cần cùng giao thức và cùng cài đặt máy chủ chuyển tiếp: **Dùng cài đặt này ở mọi nơi** đề xuất cài đặt của bạn cho các máy kia, trong **Hộp thư** của chúng.',
          },
          {
            term: 'git',
            text: 'Mã nguồn của phiên được chuyển qua git, bản 2.30 hoặc mới hơn, trên cả hai máy tính: **Cài công cụ dòng lệnh** trên máy Mac, **Cài Git for Windows** trên Windows.',
          },
          {
            term: 'Đường dẫn dài (Windows)',
            text: 'Git for Windows bỏ qua các đường dẫn dài hơn 260 ký tự, trừ khi bật đường dẫn dài. **Bật cho git** bật nó cho bạn; cài đặt riêng của Windows cần quản trị viên.',
          },
          {
            term: 'Claude Code',
            text: 'Các phiên chạy trong Claude Code, và một phiên chuyển sang phiên bản cũ hơn có thể không tiếp tục được. **Cập nhật Claude Code** chạy `claude update` khi bạn nhấn nút.',
          },
          {
            term: 'Công cụ cho Claude Code',
            text: 'Các phiên ở đây chạy lệnh trên các máy tính khác của bạn qua công cụ của Mother Claude. Đây là mục khuyến nghị; khi một máy khác cho máy này chạy lệnh trên đó, các công cụ ấy phải chờ mục này, nhưng phần còn lại vẫn hoạt động khi thiếu nó. **Bật** sẽ bật chúng.',
          },
          {
            term: 'Từng tài khoản',
            text: 'Đã đăng nhập trên máy tính này. Chỉ các phiên của tài khoản đó phải chờ: phần còn lại vẫn hoạt động khi thiếu nó. **Đăng nhập…** bắt đầu việc đăng nhập ngay tại máy này.',
          },
          {
            term: 'Dung lượng trống',
            text: 'Các lần chuyển được đặt vào thư mục dữ liệu của Mother Claude trước: nên còn trống 5 GB.',
          },
        ],
        notes: [
          'Các mục kiểm tra mạng chỉ bắt buộc khi chọn **Không dùng máy chủ chuyển tiếp**. Khi có máy chủ chuyển tiếp, chúng là khuyến nghị: các máy tính của bạn vẫn gặp nhau qua máy chủ đó, chỉ chậm hơn.',
        ],
      },
      {
        heading: 'Tài khoản và thư mục',
        intro:
          'Bạn đặt chúng trong phần thiết lập, và sau đó thay đổi trong **Máy tính › Tùy chọn › Tài khoản và thư mục**.',
        points: [
          'Tài khoản được ghép theo tên trên từng máy tính, không phân biệt chữ hoa chữ thường, và tài khoản Mặc định với tài khoản Mặc định. Một phiên được chuyển sang tài khoản cùng tên, và chỉ chạy được ở nơi tài khoản đó đã đăng nhập.',
          'Máy khác chỉ thấy tên, loại và trạng thái đăng nhập của các tài khoản của bạn: không bao giờ thấy token, email hay thư mục.',
          'Thư mục dùng chung là một thư mục trên mỗi máy tính, chẳng hạn `~/development/projects` trên máy Mac và `D:\\development\\projects` trên Windows: một dự án nằm trong nó ở máy này là cùng dự án đó, ở cùng vị trí, trên máy kia.',
          'Phần thiết lập đề xuất thư mục dùng chung dựa trên remote git của các dự án: nhấn **Dùng** để nhận một thư mục, hoặc **Thêm thư mục** để thêm thư mục của riêng bạn. Các cặp thư mục từ **Bản sao lưu** được giữ nguyên.',
          'Dự án chỉ một máy có sẽ hiện **Sao chép sang …** hoặc **Sao chép về đây**. Dự án không có remote git thì không thể chuyển cùng mã nguồn.',
          '**Đăng nhập trên …**, **Thêm trên …** và **Sao chép sang …** là những việc được nhờ máy đó: người ở máy đó sẽ xác nhận, trong **Hộp thư** của máy ấy.',
        ],
        links: [
          { label: 'Mở trang Tùy chọn', path: '/computers/options', fragment: 'accounts-folders' },
        ],
      },
      {
        heading: 'Tùy chọn',
        intro:
          'Bạn đặt chúng trong phần thiết lập, và sau đó thay đổi trong **Máy tính › Tùy chọn**: **Chung** (nhấn **Thay đổi**) và **Từng dự án**.',
        terms: [
          {
            term: 'Máy chủ chuyển tiếp',
            text: '**Máy chủ chuyển tiếp công cộng (mặc định)** dùng được ngay, ở bất cứ đâu. **Máy chủ chuyển tiếp của riêng bạn** là một `iroh-relay` do bạn tự chạy, trên NAS hoặc một máy chủ nhỏ: nhập địa chỉ `https://` của nó và, nếu muốn, SHA-256 của chứng chỉ, để chỉ đúng chứng chỉ đó được chấp nhận (kể cả chứng chỉ tự ký). **Không dùng máy chủ chuyển tiếp**: các máy tính của bạn chỉ gặp nhau trong cùng một mạng. Máy chủ chuyển tiếp chỉ truyền dữ liệu đã mã hóa.',
          },
          {
            term: 'Cách mã nguồn được chuyển khi một phiên chuyển đi',
            text: '**Trực tiếp giữa các máy tính (mặc định)**: gửi thẳng sang máy tính kia, không đẩy lên đâu cả. **Qua remote git của bạn**: các ref ẩn trên remote của bạn, bị xoá sau khi chuyển trừ khi bạn giữ lại; cả hai máy cần truy cập được remote mà không phải nhập mật khẩu.',
          },
          {
            term: 'Nơi mã nguồn được đặt',
            text: '**Trong thư mục dự án, khi nó không có thay đổi riêng (mặc định)**, hoặc **Luôn ở một thư mục riêng (worktree), không bao giờ ở thư mục dự án**: một worktree trong thư mục dữ liệu của Mother Claude trên máy nhận phiên.',
          },
          {
            term: 'Giữ máy thức',
            text: 'Khi một máy tính khác đang dùng máy này, Mother Claude giữ máy thức. Không cài đặt hệ thống nào bị thay đổi.',
          },
        ],
        points: [
          'Hãy dùng cùng máy chủ chuyển tiếp trên mọi máy (hai máy dùng máy chủ chuyển tiếp khác nhau có thể không bao giờ gặp được nhau), và tốt nhất là cùng tùy chọn mã nguồn: thay đổi ở đây được đề xuất cho các máy kia, trong **Hộp thư** của chúng, và mỗi máy nhận khi người ở máy đó nhấn **Dùng cả ở đây** (hoặc **Giữ cài đặt của máy này**). Một lần chuyển theo tùy chọn của máy bắt đầu chuyển.',
          '**Từng dự án** có thể chuyển mã nguồn theo cách khác, và có một công thức: **Sau khi chuyển, nhờ Claude chạy**, mỗi dòng một lệnh, cho macOS, Windows và Linux. Các lệnh được gửi cho Claude, và Claude chạy chúng qua các bước phê duyệt của chính phiên đó. **Các tệp git bỏ qua sẽ đi kèm**, chẳng hạn `.env.local`, đi cùng mỗi lần chuyển của dự án: chúng thường chứa bí mật, nên việc thêm tệp, và việc mang chúng theo, đều cần quyền thao tác nguy hiểm.',
          '**Kiểm tra remote** xem remote git của một dự án có phản hồi mà không yêu cầu mật khẩu trên từng máy không. Việc kiểm tra chỉ chạy khi bạn nhấn nút, và chạy lại trước mỗi lần chuyển qua remote.',
        ],
        links: [{ label: 'Mở trang Tùy chọn', path: '/computers/options' }],
      },
      {
        heading: 'Trang Máy tính, từng thẻ một',
        intro: 'Khi các máy tính của bạn đã được thiết lập, trang **Máy tính** có bốn thẻ:',
        terms: [
          {
            term: 'Tổng quan',
            text: '**Máy Mac này** (hoặc **Máy PC này**), rồi một thẻ cho mỗi máy đã ghép nối: cách nó đang kết nối, các phiên bản của nó, các phiên của nó (**Mở các phiên của máy này**), các mục kiểm tra không đạt kèm cách sửa, quyền của mỗi máy trên máy kia (**Thay đổi**), **Đổi tên**, **Thu hồi** và **Thu hồi ở mọi nơi**. Sau đó là **Ghép nối các máy tính của bạn với nhau** và **Bản sao**.',
          },
          {
            term: 'Đồng bộ mã nguồn',
            text: 'Các dự án được giữ giống nhau trên hai máy tính, và **Giữ một dự án đồng bộ…**.',
          },
          {
            term: 'Hoạt động',
            text: 'Những gì đã diễn ra giữa các máy tính của bạn: **Các lần chuyển gần đây**, **Các lần chạy**, **Tác vụ** và **Vai trò trong nhóm**. Các nút lọc ở trên cùng chỉ hiện một loại. Các lệnh được phép chạy mà không hỏi nằm trong **Quyền**.',
          },
          {
            term: 'Tùy chọn',
            text: '**Chung** (máy chủ chuyển tiếp, cách chuyển mã nguồn, bản sao, giữ máy thức), **Tài khoản và thư mục** và **Từng dự án**, rồi **Chạy lại thiết lập** và **Tắt tính năng Máy tính**.',
          },
        ],
        points: [
          '**Máy Mac này** (hoặc **Máy PC này**) hiện các phiên bản của máy, **Công cụ cho Claude Code** có đang bật không, các mục kiểm tra không đạt kèm cách sửa, và lần kiểm tra gần nhất: **Kiểm tra lại** chạy chúng ngay. Trên máy Mac, thẻ này còn hiện nhanh các quyền macOS: **Quyền** mở danh sách các quyền.',
          '**Dịch vụ nền của Claude Code** cho biết dịch vụ có đang chạy không và có bao nhiêu công việc nền. **Chi tiết** hiện dịch vụ đó, cùng các máy chủ MCP đã cấu hình cho Claude Code và các công việc nền.',
          'Những gì các máy tính khác yêu cầu máy này, một máy tính mà một trong số chúng giới thiệu và một cài đặt mà một máy đề xuất đều chờ trong **Hộp thư**.',
          'Trên điện thoại, **Máy tính** nằm trong **Thêm**: việc ghép nối, thiết lập, quyền và tùy chọn được thay đổi trong ứng dụng máy tính.',
        ],
        links: [
          { label: 'Mở trang Đồng bộ mã nguồn', path: '/computers/sync' },
          { label: 'Mở trang Hoạt động', path: '/computers/activity' },
        ],
      },
    ],
  },
  {
    id: 'machines-pair',
    title: 'Ghép nối các máy tính của bạn',
    icon: 'key',
    group: 'machines',
    summary:
      'Ghép nối hai máy tính bằng một mã dùng một lần, và kiểm tra cùng sáu chữ số trên cả hai máy.',
    intro:
      'Hai máy tính chỉ cần ghép nối một lần, trong ứng dụng máy tính của mỗi máy: trong cùng mạng hoặc qua Internet, không cần mở cổng hay chạy đường hầm.',
    keywords: [
      'ghép nối',
      'tham gia',
      'mã',
      'ở gần',
      'sáu chữ số',
      'thu hồi',
      'hủy ghép nối',
      'đổi tên',
      'khóa máy',
    ],
    sections: [
      {
        heading: 'Ghép nối hai máy tính',
        where: 'desktop',
        steps: [
          'Trên máy tính thứ nhất, mở **Máy tính** và nhấn **Ghép nối một máy tính**. Một mã 8 ký tự hiện ra: mã dùng được một lần, trong năm phút.',
          'Trên máy tính kia, mở **Máy tính** và nhấn **Tham gia…**.',
          'Trong **Tham gia ghép nối với máy tính khác**, nhập mã vào **Mã** và nhấn **Tham gia**.',
          'Kiểm tra rằng cả hai màn hình hiện cùng sáu chữ số.',
          'Trên mỗi máy, chọn những gì máy kia được làm trên máy này, rồi nhấn **Dãy số khớp — Ghép nối**.',
        ],
        tips: [
          '**Ở gần** liệt kê các máy tính trong mạng của bạn đang hiện mã, để bạn kiểm tra máy nào đang hiện mã: bạn vẫn phải nhập mã của máy đó, vì việc tham gia cần mã. Không có máy nào ở gần? Mã dùng được cả qua Internet.',
          'Phần thiết lập có cùng các nút này ở bước **Các máy tính của bạn**. Trước khi thiết lập, trang **Máy tính** chỉ có **Thiết lập các máy tính của bạn**.',
        ],
        links: [{ label: 'Mở trang Máy tính', path: '/computers' }],
      },
      {
        heading: 'Qua Internet',
        points: [
          'Khi máy kia tham gia qua Internet, máy thứ nhất không hiện các chữ số: hãy nhập sáu chữ số mà máy kia hiển thị, rồi nhấn **Ghép nối**.',
          'Nhập sai một lần là lượt ghép nối kết thúc, và chưa có gì được ghép nối. Hãy bắt đầu lại với mã mới.',
        ],
      },
      {
        heading: 'Vì sao an toàn',
        points: [
          'Chỉ người có mã mới bắt đầu được việc ghép nối: mã giúp tìm máy kia, và chứng minh bạn đang giữ mã.',
          'Sáu chữ số được tính từ khóa của cả hai máy, nên một máy chen vào giữa, kể cả máy đã thấy mã, sẽ làm hai màn hình hiện hai dãy số khác nhau.',
          'Không có gì được lưu cho đến khi cả hai máy xác nhận.',
          'Máy đã ghép nối được nhận ra chỉ bằng khóa của nó, lưu trong Chuỗi khóa hoặc Windows Credential Manager. Cài lại Mother Claude sẽ tạo khóa mới: hãy ghép nối lại máy đó.',
        ],
      },
      {
        heading: 'Đổi tên hoặc thu hồi một máy',
        where: 'desktop',
        points: [
          'Trên **Máy tính**, thẻ của mỗi máy đã ghép nối có **Đổi tên** (tên của nó trên máy này) và **Thu hồi**.',
          '**Thu hồi**, rồi **Nhấn lần nữa để thu hồi**: máy này quên máy kia và cắt các kết nối của nó ngay lập tức.',
          '**Thu hồi** chỉ có tác dụng trên máy này. Với một máy bị mất hoặc bị đánh cắp, hãy dùng **Thu hồi ở mọi nơi** trên bất kỳ máy nào khác của bạn: máy đó bị cắt khỏi tất cả các máy.',
          '**Tắt tính năng Máy tính** (**Máy tính › Tùy chọn**) không hủy ghép nối, nhưng khi tính năng này tắt, máy đã ghép nối chỉ ghép nối và chạy thiết lập được với máy này: nó không xem, điều khiển hay chuyển được các phiên ở đây, và máy này cũng không dùng các máy kia. **Thu hồi** để quên một máy.',
        ],
      },
    ],
  },
  {
    id: 'machines-more',
    title: 'Ba máy tính trở lên',
    icon: 'monitor',
    group: 'machines',
    summary:
      'Ghép nối một máy tính mới với các máy khác qua một máy bạn đã có, cắt một máy bị mất khỏi tất cả, và giữ phiên bản các máy đồng đều.',
    intro:
      'Mỗi cặp máy tính của bạn được ghép nối riêng: một máy có thể giới thiệu hai máy khác với nhau, nhưng không có gì được tin cậy thay cho chúng. Việc thu hồi một máy bị mất ở mọi nơi, và phiên bản mỗi máy đang chạy, cũng nằm trên **Máy tính**.',
    keywords: [
      'máy thứ ba',
      'giới thiệu',
      'lời giới thiệu',
      'thu hồi ở mọi nơi',
      'bị mất',
      'bị đánh cắp',
      'phiên bản',
      'cập nhật',
      'cũ hơn',
    ],
    sections: [
      {
        heading: 'Ghép nối một máy tính mới với các máy khác',
        where: 'desktop',
        intro:
          'Trước tiên hãy ghép nối máy mới với một máy của bạn (**Ghép nối các máy tính của bạn**). Máy đó sau đó sẽ đề nghị **Ghép nối thêm … với …?** cho từng máy khác của bạn chưa được ghép nối với máy mới, ngay sau khi ghép nối và trên **Máy tính**, trong **Ghép nối các máy tính của bạn với nhau**.',
        steps: [
          'Nhấn **Giới thiệu chúng với nhau**. Cả hai máy được báo máy kia là ai; chưa có gì được ghép nối.',
          'Trên mỗi máy trong hai máy, mở **Hộp thư**: trong **Cần bạn**, **… giới thiệu …** cho biết máy nào đã giới thiệu máy nào. Nhấn **Ghép nối với …**.',
          'Kiểm tra rằng cả hai màn hình hiện cùng sáu chữ số. Khi một máy kết nối qua Internet, máy kia sẽ bảo bạn nhập các chữ số đó, như khi ghép nối bằng mã.',
          'Trên mỗi máy, chọn những gì máy kia được làm trên máy này, rồi nhấn **Dãy số khớp — Ghép nối**.',
        ],
        tips: [
          '**Để sau** trên bất kỳ máy nào sẽ kết thúc việc đó ở máy ấy và báo cho máy kia: không có gì được ghép nối. Trong lúc hai máy ghép nối, **Hiện việc ghép nối** mở lại phần ghép nối.',
          'Một lời giới thiệu chờ cả hai câu trả lời trong mười phút: quá thời gian đó, hãy giới thiệu chúng lại.',
          'Phần thiết lập có cùng các đề nghị này ở bước **Các máy tính của bạn**.',
        ],
        links: [
          { label: 'Mở trang Máy tính', path: '/computers' },
          { label: 'Mở trang Hộp thư', path: '/inbox' },
        ],
      },
      {
        heading: 'Vì sao lời giới thiệu an toàn',
        points: [
          'Máy giới thiệu chỉ chuyển tên, mã định danh, khóa công khai và các địa chỉ gần nhất của mỗi máy: không bao giờ chuyển bí mật, và không bao giờ chuyển những gì mỗi máy được làm.',
          'Mỗi máy trong hai máy kết nối tới máy kia bằng đúng khóa đã được báo, và không gì khác vào được qua lời giới thiệu đó.',
          'Sáu chữ số được tính từ khóa của cả hai máy, như khi ghép nối bằng mã: một máy chen vào giữa sẽ làm hai màn hình hiện hai dãy số khác nhau.',
          'Mỗi ứng dụng máy tính tự quyết định và tự xác nhận; chưa đủ cả hai thì không có gì được lưu. Sự tin cậy không bao giờ được truyền tiếp: được ghép nối với một máy không cho bạn quyền gì trên các máy mà máy đó được ghép nối.',
        ],
      },
      {
        heading: 'Thu hồi một máy ở mọi nơi',
        where: 'desktop',
        intro:
          'Dành cho máy bị mất hoặc bị đánh cắp: cắt nó khỏi tất cả các máy của bạn từ bất kỳ máy nào.',
        steps: [
          'Trên **Máy tính**, tìm thẻ của máy đó và nhấn **Thu hồi ở mọi nơi**.',
          'Nhấn **Nhấn lần nữa để thu hồi ở mọi nơi**.',
        ],
        points: [
          'Máy này quên máy đó và cắt các kết nối của nó ngay lập tức, rồi đề nghị từng máy khác của bạn làm như vậy. Mỗi máy làm ngay, và hiện thông báo rằng bạn đã thu hồi nó trên tất cả máy tính của bạn.',
          'Một máy đang tắt hoặc ở xa sẽ làm khi kết nối lại lần tới, và đến lúc đó vẫn cho nó vào: **Hộp thư** cho biết những máy nào còn phải làm, trong **Cần sửa** (**… chưa bị thu hồi ở mọi nơi**). **Đóng thông báo** cất mục này đi.',
          'Việc này chỉ gỡ bỏ sự tin cậy: nó không thêm gì ở đâu cả, và không máy nào dùng được nó để thu hồi chính máy đã đề nghị.',
        ],
      },
      {
        heading: 'Phiên bản',
        points: [
          'Thẻ của mỗi máy trên **Máy tính** hiện hệ điều hành, phiên bản Mother Claude, giao thức và Claude Code của máy đó.',
          'Một máy chạy Mother Claude cũ hơn sẽ báo như vậy, kèm **Nhờ … cập nhật**: máy đó sẽ hiện trang **Cập nhật phần mềm** khi có người ở đó xác nhận. Khi máy này là máy cũ hơn, **Cập nhật** mở **Cập nhật phần mềm** ở đây.',
          'Một máy cũ hơn cũng cho biết những gì nó chưa làm được cho đến khi được cập nhật, như **nhận tác vụ** hay **giữ mã nguồn đồng bộ**. Hai máy dùng giao thức khác nhau hoàn toàn không làm việc được với nhau cho đến khi một máy cập nhật.',
          'Claude Code cũ hơn ở máy kia cũng có **Nhờ … cập nhật**; ở đây, **Cập nhật** chạy `claude update`.',
        ],
      },
    ],
  },
  {
    id: 'machines-permissions',
    title: 'Máy khác được làm gì',
    icon: 'shield',
    group: 'machines',
    summary: 'Các quyền mỗi máy tính cấp cho các máy khác, trong cùng mạng và qua Internet.',
    intro:
      'Mỗi máy tính tự quyết định từng máy khác được làm gì trên nó. Bạn chọn khi ghép nối, và có thể đổi sau trên trang **Máy tính**.',
    keywords: [
      'quyền',
      'cấp quyền',
      'xem',
      'điều khiển',
      'di chuyển',
      'chạy lệnh',
      'nguy hiểm',
      'nhật ký kiểm tra',
      'cùng mạng',
    ],
    sections: [
      {
        heading: 'Các quyền',
        terms: [
          {
            term: 'Xem',
            text: 'Liệt kê các phiên, đọc bản ghi, xem yêu cầu duyệt và câu hỏi. Bật theo mặc định.',
          },
          {
            term: 'Điều khiển',
            text: 'Gửi tin nhắn, trả lời yêu cầu duyệt thông thường, ngắt, dừng, và đặt mô hình, mức nỗ lực và chế độ. Bật theo mặc định.',
          },
          {
            term: 'Di chuyển',
            text: 'Chuyển phiên đến máy này và đi khỏi máy này. Bật theo mặc định.',
          },
          {
            term: 'Chạy',
            text: 'Chạy lệnh ở đây cho các phiên của máy đó, trong một thư mục làm việc riêng, và nhận các tác vụ chúng giao, trong một worktree riêng: bản sao dự án của máy này không bao giờ bị động đến. Mỗi việc đều được duyệt trên máy kia. Tắt theo mặc định.',
          },
          {
            term: 'Nguy hiểm',
            text: 'Bỏ qua quyền, cho phép các lệnh gọi công cụ bị đánh dấu nguy hiểm, gửi phím thô, mang theo sự tin cậy của thư mục cùng các tệp git bỏ qua khi chuyển phiên, và chạy các lệnh bị đánh dấu nguy hiểm. Tắt theo mặc định.',
          },
        ],
        points: [
          'Điều khiển cần quyền xem; di chuyển, chạy lệnh và thao tác nguy hiểm cần quyền điều khiển. Đánh dấu một quyền sẽ đánh dấu cả những quyền nó cần; bỏ một quyền sẽ bỏ cả những quyền cần nó.',
        ],
        links: [
          { label: 'Xem “Chạy lệnh trên máy tính khác”', path: '/help/machines-run' },
          { label: 'Xem “Giao tác vụ cho máy tính khác”', path: '/help/machines-task' },
        ],
      },
      {
        heading: 'Cùng mạng hay qua Internet',
        points: [
          'Mỗi quyền có hai cột: **Cùng mạng**, cho kết nối trực tiếp trong mạng cục bộ của bạn, và **Internet**, cho mọi trường hợp khác, kể cả qua máy chủ chuyển tiếp.',
          'Thẻ của một máy trên **Máy tính** cho biết hiện nó đang kết nối thế nào: **Trực tiếp (cùng mạng)**, **Trực tiếp (Internet)** hoặc **Qua máy chủ chuyển tiếp (chậm hơn)**.',
        ],
      },
      {
        heading: 'Thay đổi quyền',
        where: 'desktop',
        steps: [
          'Mở **Máy tính**.',
          'Trên thẻ của máy kia, cạnh **Trên máy Mac này, … được** (hoặc **Trên máy PC này, … được**), nhấn **Thay đổi**.',
          'Đánh dấu hoặc bỏ đánh dấu các ô, rồi nhấn **Lưu**.',
        ],
        points: [
          '**Trên …, máy Mac này được** cho biết máy kia cho máy này làm gì. Chỉ máy kia mới thay đổi được.',
          'Phần thiết lập có cùng bảng này ở bước **Quyền của từng máy**.',
        ],
        links: [{ label: 'Mở trang Máy tính', path: '/computers' }],
      },
      {
        heading: 'Thao tác nguy hiểm qua máy khác',
        points: [
          'Một thao tác nguy hiểm trên máy khác cần cả hai: quyền **Nguy hiểm** mà máy đó cấp cho máy này, và quyền của chính bạn ở đây. Một điện thoại không có quyền thao tác nguy hiểm trên máy này không thể dùng máy này để làm việc đó. Một lần chuyển bạn bắt đầu từ đây cho phiên của máy khác sẽ quay về máy này từ máy đó: khi đó việc mang theo sự tin cậy của thư mục hay các tệp git bỏ qua cần quyền **Nguy hiểm** mà mỗi máy cấp cho máy kia.',
          'Mọi thay đổi mà máy khác thực hiện ở đây đều được ghi vào nhật ký kiểm tra của máy này, kèm người đã thao tác qua máy đó; thao tác nguy hiểm còn hiện thông báo ở đây.',
          'Có quyền **Nguy hiểm**, máy kia có thể chạy bất cứ thứ gì ở đây: chỉ cấp cho máy tính bạn tin cậy như chính máy này.',
          'Có quyền **Chạy**, máy kia chạy lệnh và khởi động phiên Claude Code ở đây với tư cách của bạn, mỗi việc được duyệt trên máy đó chứ không phải trên máy này: chỉ cấp cho máy tính mà bạn sẵn lòng để nó gõ lệnh vào terminal ở đây.',
        ],
      },
    ],
  },
  {
    id: 'machines-sessions',
    title: 'Dùng phiên của máy khác',
    icon: 'sessions',
    group: 'machines',
    summary:
      'Xem và điều khiển phiên của mọi máy từ bất kỳ máy nào, và bắt đầu phiên trên máy khác.',
    intro:
      'Sau khi hai máy tính được ghép nối và cả hai đã bật tính năng Máy tính, mỗi máy liệt kê các phiên Claude Code của máy kia. Mở một phiên và dùng nó như thể nó chạy ở đây.',
    keywords: [
      'máy khác',
      'bộ lọc máy tính',
      'nhãn máy',
      'hộp thư',
      'ngoại tuyến',
      'nơi chạy',
      'phiên mới',
      'điện thoại',
      'điều khiển từ xa',
    ],
    sections: [
      {
        heading: 'Tìm các phiên',
        points: [
          '**Phiên** liệt kê phiên của mọi máy. Phiên của máy khác có nhãn ghi tên máy đó; phiên của máy này thì không.',
          'Bộ lọc **Máy tính** chọn một máy: **Mọi máy tính**, **máy Mac này** (hoặc **máy PC này**), hoặc một máy khác theo tên.',
          'Trên **Máy tính**, thẻ của một máy cho biết máy đó có bao nhiêu phiên và bao nhiêu phiên đang chạy; **Mở các phiên của máy này** liệt kê chúng.',
          '**Hộp thư** liệt kê những gì đang chờ bạn ở mọi máy, mỗi yêu cầu kèm máy của nó. Hãy trả lời ở đây: câu trả lời đầu tiên được tính, như với điện thoại.',
        ],
        links: [{ label: 'Mở trang Phiên', path: '/' }],
      },
      {
        heading: 'Điều khiển các phiên',
        points: [
          'Mở một phiên: trang của nó ghi **Chạy trên … — bạn điều khiển nó từ đây.** Ô nhập tin nhắn, yêu cầu duyệt, dừng, mô hình, mức nỗ lực và chế độ hoạt động như với phiên của máy này.',
          'Khi máy kia không cấp quyền **Điều khiển**, trang ghi **Chạy trên … — bạn có thể xem nó từ đây.**, và việc bạn thử làm sẽ cho biết lý do, chẳng hạn **… chỉ cho máy Mac này xem các phiên của nó, không cho điều khiển.**',
          'Việc đổi tài khoản, chuyển hoặc sao chép phiên sang tài khoản khác hay kho lưu trữ, xóa phiên, mở tệp của phiên và mở phiên trong VS Code được thực hiện trên máy tính mà phiên đang chạy.',
        ],
      },
      {
        heading: 'Bắt đầu phiên trên máy khác',
        steps: [
          'Mở **Phiên mới**.',
          'Trong **Nơi chạy**, chọn máy kia.',
          'Chọn một dự án, hoặc nhập đường dẫn vào **Hoặc bất kỳ thư mục nào trên …** (ứng dụng máy tính).',
          'Nhấn **Bắt đầu trên …**.',
        ],
        points: [
          'Phiên chạy với tài khoản của máy đó cho thư mục này, và bạn điều khiển nó từ đây.',
          'Claude Code trên máy đó phải tin cậy thư mục này rồi: hãy mở nó bằng Claude Code trên máy đó một lần.',
        ],
        links: [{ label: 'Mở trang Phiên mới', path: '/new' }],
      },
      {
        heading: 'Khi một máy ngoại tuyến',
        points: [
          'Các phiên của máy đó vẫn được liệt kê, làm mờ, trong **Trên các máy tính đang ngoại tuyến**, cho đến khi máy trở lại.',
          'Trang của phiên cho biết máy của nó ngoại tuyến từ lúc nào. Những gì bạn thấy là từ lúc đó, và việc gửi sẽ chờ đến khi máy trở lại.',
          'Máy tính đang ngủ không trả lời được: **Giữ máy thức khi đang được dùng** giữ từng máy thức khi có máy khác đang dùng nó.',
        ],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Điện thoại đã ghép nối với bất kỳ máy tính nào của bạn sẽ xem và điều khiển được mọi máy thông qua máy đó.',
          'Thao tác nguy hiểm trên máy khác cần quyền thao tác nguy hiểm của điện thoại trên máy mà nó ghép nối, và quyền **Nguy hiểm** mà máy kia cấp cho máy đó.',
          'Điện thoại ở trạng thái **Hạn chế** (tự làm mới vài giây một lần thay vì cập nhật trực tiếp) vẫn thấy các phiên của các máy khác, trong cùng lần làm mới đó.',
          'Việc thiết lập các máy tính, ghép nối chúng và thay đổi quyền của chúng được thực hiện trong ứng dụng máy tính; trên điện thoại, **Máy tính** nằm trong **Thêm**.',
        ],
      },
    ],
  },
  {
    id: 'machines-move',
    title: 'Chuyển phiên sang máy tính khác',
    icon: 'move',
    group: 'machines',
    summary:
      'Tiếp tục một phiên trên máy tính khác, cùng mã nguồn của nó, rồi đưa về hoặc hoàn tác.',
    intro:
      'Một phiên có thể tiếp tục trên một máy tính khác của bạn: cùng cuộc trò chuyện, cùng mã nguồn, tiếp nối đúng chỗ đang dừng. Mỗi lúc phiên chỉ ở trên một máy; máy mà phiên rời đi giữ lại bản của nó, chỉ để đọc, và cho biết phiên giờ đang ở đâu.',
    keywords: [
      'chuyển phiên',
      'tiếp tục trên máy khác',
      'đưa về',
      'tiếp quản',
      'hoàn tác',
      'công thức',
      'mã nguồn',
      'đang ở trên',
    ],
    sections: [
      {
        heading: 'Chuyển phiên',
        steps: [
          'Mở menu **⋮** của phiên và chọn **Tiếp tục trên máy tính khác…**.',
          'Trong **Máy tính**, chọn nơi phiên sẽ đến. Mother Claude hỏi máy đó trước và hiện những gì nó tìm thấy: chưa có gì được ghi ở đâu cả.',
          'Xem lại **Những gì được chuyển**, **Nơi đến**, **Cách chuyển mã nguồn**, **Nơi đặt mã nguồn** và phần **Kiểm tra**.',
          'Nhấn **Tiếp tục trên …**.',
        ],
        tips: [
          'Với phiên của máy khác, menu có **Đưa về máy Mac này** (hoặc **Đưa về máy PC này**), hoặc **Tiếp tục trên máy Mac này…** cho phiên chưa từng ở đây. Một lần chuyển luôn bắt đầu trên máy mà phiên đang ở: từ đây, Mother Claude nhờ máy đó thực hiện.',
        ],
      },
      {
        heading: 'Các lựa chọn',
        terms: [
          {
            term: 'Máy tính',
            text: 'Các máy tính đã ghép nối đang trực tuyến và cho máy này chuyển phiên sang. Máy đang ngoại tuyến cho biết từ lúc nào.',
          },
          {
            term: 'Những gì được chuyển',
            text: 'Dung lượng cuộc trò chuyện, kèm các tác tử con, và mã nguồn: nhánh, số tệp đã thay đổi và số tệp mới.',
          },
          {
            term: 'Ngoài git',
            text: 'Các tệp git bỏ qua mà dự án liệt kê (**Máy tính › Tùy chọn › Từng dự án**), chẳng hạn `.env.local`. Chúng thường chứa bí mật, nên chỉ được mang theo khi có quyền thao tác nguy hiểm ở đây và quyền **Nguy hiểm** của máy nhận; bỏ đánh dấu **Mang theo chúng** để để chúng lại. Tệp cùng tên ở máy đó được giữ lại cho **Hoàn tác việc chuyển**.',
          },
          {
            term: 'Nơi đến',
            text: 'Thư mục của dự án trên máy kia, tìm qua thư mục dùng chung, cặp thư mục hoặc cùng đường dẫn. **Đổi…** để chọn thư mục khác; khi không tìm thấy, **Chọn thư mục của nó trên …** liệt kê các dự án của máy đó.',
          },
          {
            term: 'Cách chuyển mã nguồn',
            text: '**Trực tiếp giữa các máy tính** (mặc định: thẳng giữa hai máy, không đẩy lên đâu cả), **Qua remote git của bạn** (ref ẩn trên remote của bạn, bị xoá sau đó trừ khi được giữ lại) hoặc **Chỉ cuộc trò chuyện**. Qua remote, remote phải phản hồi trên cả hai máy mà không yêu cầu mật khẩu: việc này được kiểm tra trước khi chuyển. Khi gửi trực tiếp giữa các máy tính, mã nguồn lớn hơn 1 GB sẽ tự đi qua remote của bạn nếu remote phản hồi trên cả hai máy, và việc chuyển sẽ báo điều đó.',
          },
          {
            term: 'Nơi đặt mã nguồn',
            text: '**Trong thư mục dự án** (mặc định, khi ở đó nó không có thay đổi riêng) hoặc **Trong thư mục riêng (worktree)**: một worktree của nó trong thư mục dữ liệu của Mother Claude ở máy đó; khi đó thư mục dự án không bao giờ bị đụng đến, và không có gì được gộp vào nó.',
          },
          {
            term: 'Sau khi chuyển',
            text: 'Khi dự án có công thức (đặt trong **Máy tính › Tùy chọn › Từng dự án**, hoặc tệp `.mother-claude/recipe.json` được commit trong dự án): nhờ Claude chạy các lệnh của nó trên máy kia, chẳng hạn `npm ci`. Chúng nằm trong tin nhắn đầu tiên Claude nhận ở đó và đi qua các yêu cầu duyệt của chính nó: Mother Claude không bao giờ tự chạy lệnh trên máy kia.',
          },
          {
            term: 'Khi nào',
            text: 'Khi một lượt đang chạy: **Sau lượt này** (mặc định: để lượt chạy xong trước) hoặc **Ngay bây giờ** (ngắt lượt hiện tại; những gì bạn đã xếp hàng đợi sẽ đi kèm).',
          },
          {
            term: 'Tiếp tục điều khiển nó từ đây',
            text: 'Bật theo mặc định: sau khi chuyển, trang này trở thành phiên ở nơi nó đang chạy.',
          },
          {
            term: 'Mang theo sự tin cậy của thư mục này sang …',
            text: 'Hiện khi Claude Code trên máy kia chưa từng mở thư mục đó. Thư mục được tin cậy sẽ chạy hook và máy chủ MCP của dự án mà không hỏi, nên đây là thao tác nguy hiểm: cần ứng dụng máy tính hoặc thiết bị được phép thực hiện thao tác nguy hiểm, và quyền **Nguy hiểm** mà máy nhận phiên cấp cho máy phiên rời đi.',
          },
        ],
      },
      {
        heading: 'Các mục kiểm tra',
        points: [
          '**Kiểm tra** liệt kê những gì ổn, những gì chặn việc chuyển kèm cách sửa, và những gì chỉ làm nó kém đi (Claude Code ở máy kia cũ hơn, kiểu xuống dòng, Git LFS, submodule). Cả hai máy phải bật tính năng Máy tính.',
          'Khi còn mục chặn, nút ghi **Hãy khắc phục các mục kiểm tra trước**. Khi thư mục dự án ở máy kia có thay đổi riêng, hoặc nhánh của nó đã đi tiếp ở đó, Mother Claude đề xuất **Dùng thư mục riêng (worktree)**.',
          'Khi nhánh của nó đã đi tiếp ở máy kia, Mother Claude còn đề xuất thay nhánh đó bằng nhánh của máy này: các commit riêng của nhánh ở đó được giữ lại, và **Hoàn tác việc chuyển** sẽ đưa chúng trở lại. Nếu nhánh ở đó đi tiếp trước khi việc chuyển hoàn tất, việc chuyển sẽ dừng.',
          'Phiên do ứng dụng khác chạy (VS Code, terminal) không thể bị Mother Claude dừng: một bản sao tiếp tục ở máy kia dưới dạng phiên mới, trong khi phiên này vẫn tiếp tục ở đây. Nút ghi **Tiếp tục dưới dạng bản sao trên …**; hoặc hãy kết thúc nó trong ứng dụng đó trước.',
        ],
      },
      {
        heading: 'Trong lúc chuyển',
        points: [
          'Từng bước hiện kèm thời gian: đã kiểm tra, đã dừng ở đây, đã đóng gói, đã gửi (bao nhiêu, và còn khoảng bao lâu), đã đặt lên máy kia, đã khởi động ở đó.',
          '**Hủy việc chuyển** dùng được cho đến khi phiên bắt đầu được đặt lên máy kia. **Đóng (vẫn tiếp tục chạy)** để việc chuyển tiếp tục; trang của phiên cho biết nó đang làm gì trong lúc đó.',
          'Kết nối bị rớt trong chốc lát khi đang gửi sẽ tiếp tục từ chỗ đã dừng.',
          'Tin nhắn đầu tiên phiên nhận ở máy kia cho Claude biết nó đã được chuyển: từ máy và thư mục nào sang đâu, rằng đường dẫn trong các tin nhắn trước là của máy cũ, và giờ nó dùng shell nào. Bản thân cuộc trò chuyện không bao giờ bị viết lại.',
          'Những điều cần biết được liệt kê dưới các bước và trong **Các lần chuyển gần đây**: một tệp riêng mà máy kia đã có, được giữ lại để **Hoàn tác việc chuyển**, các cảnh báo của phần kiểm tra và của mã nguồn (Git LFS, submodule, quyền thực thi mà Windows làm mất…).',
        ],
      },
      {
        heading: 'Nếu có lỗi',
        terms: [
          {
            term: 'Trước khi được đặt lên máy kia',
            text: 'Không mất gì: phiên tiếp tục ở đây như trước (và khởi động lại nếu nó đang chạy), và máy kia bỏ những gì đã nhận. Mother Claude cho biết lý do; **Thử lại** đưa bạn về các lựa chọn.',
          },
          {
            term: 'Mã nguồn không thể đặt vào thư mục dự án',
            text: 'Mother Claude đề xuất một thư mục riêng (worktree) và hỏi lại.',
          },
          {
            term: 'Đã đặt lên nhưng chưa khởi động được ở đó',
            text: 'Chẳng hạn tài khoản của nó chưa đăng nhập ở đó. Giờ phiên đang ở máy đó: **Thử lại** khởi động nó ở đó mà không gửi lại gì; **Hoàn tác việc chuyển** đưa nó về.',
          },
        ],
      },
      {
        heading: 'Đang ở trên …',
        intro:
          'Máy mà phiên rời đi giữ lại bản của nó, làm mờ trong danh sách với nhãn **Đang ở trên …**. Trang của phiên cho biết phiên đang ở đâu và từ lúc nào, cùng các nút:',
        terms: [
          { term: 'Mở ở đó', text: 'Mở phiên ở nơi nó đang chạy.' },
          {
            term: 'Đưa nó về đây',
            text: 'Một lần chuyển về máy này, bắt đầu trên máy mà phiên đang ở, khi máy đó trực tuyến. Chỉ những gì đã diễn ra ở đó được thêm vào: bản trên máy này là phần đầu của cùng cuộc trò chuyện.',
          },
          {
            term: 'Tiếp quản…',
            text: 'Chỉ khi máy mà phiên đang ở ngoại tuyến: phiên tiếp tục ở đây từ bản của máy này, bản này chỉ có cuộc trò chuyện đến lúc nó được chuyển đi, với thư mục dự án của máy này như hiện tại. Nếu trong lúc đó máy kia vẫn tiếp tục phiên, cả hai phiên bản sẽ được giữ song song khi nó được chuyển sang đó lần tới.',
          },
        ],
        points: ['Không thể gửi tin nhắn tới bản ở đây; trang của phiên cho biết lý do.'],
      },
      {
        heading: 'Hoàn tác việc chuyển',
        steps: [
          'Tìm lần chuyển: ngay khi chuyển xong, trong **Các lần chuyển gần đây** trên **Máy tính › Hoạt động**, hoặc trong **Các lần chuyển giữa máy tính** ở thẻ **Thông tin** của phiên.',
          'Nhấn **Hoàn tác việc chuyển**.',
          'Nhấn **Chạm lần nữa để hoàn tác**.',
        ],
        points: [
          'Có thể hoàn tác trong bảy ngày. Việc hoàn tác đưa thư mục dự án và nhánh trên máy kia về như cũ, gỡ các tệp của cuộc trò chuyện ở đó ra, và phiên lại ở đây (khởi động lại ở đây nếu nó đang chạy).',
          'Không thể hoàn tác khi phiên đã tiếp tục ở máy kia: hãy đưa nó về, để không mất những gì nó đã làm ở đó. Bản sao thì không bao giờ được hoàn tác: hãy dừng hoặc xóa nó ở máy kia.',
        ],
        links: [{ label: 'Mở trang Hoạt động', path: '/computers/activity' }],
      },
      {
        heading: 'Những gì không bao giờ được chuyển',
        points: [
          'Các tệp git bỏ qua, như tệp `.env`, `node_modules` và kết quả build, vẫn ở nguyên chỗ, trừ vài tệp mà dự án liệt kê để mang theo (**Ngoài git**). Hãy thiết lập dự án ở máy kia bằng công thức.',
          'Thông tin đăng nhập: token, khóa và `.credentials.json` ở lại trên từng máy. Phiên chạy với tài khoản cùng tên ở máy kia, đã đăng nhập ở đó.',
          'Worktree mà một lần chuyển tạo ra trên máy kia, trong thư mục dữ liệu của Mother Claude ở đó, vẫn ở lại khi phiên rời đi; công việc của nó đã đi cùng phiên. Khi phiên đã rời đi hoặc đã kết thúc ở đó, **Gỡ bỏ worktree** trên lần chuyển (**Các lần chuyển gần đây**, thẻ **Thông tin** của phiên) sẽ xoá nó, từ bất kỳ máy nào. Việc này bị từ chối khi trong đó còn thứ chưa nằm trong bản chụp nào của phiên: hãy chuyển phiên đi trước.',
        ],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Điện thoại đã ghép nối với bất kỳ máy tính nào của bạn cũng chuyển phiên theo cùng cách, trên một cột.',
          'Việc mang theo sự tin cậy của thư mục hay các tệp git bỏ qua cần quyền thao tác nguy hiểm của điện thoại, được đặt trong ứng dụng máy tính; không có quyền này thì tùy chọn sự tin cậy không hiện, và các tệp đó ở lại chỗ cũ.',
        ],
      },
    ],
  },

  {
    id: 'machines-copies',
    title: 'Giữ bản sao trên một máy tính khác',
    icon: 'copy',
    group: 'machines',
    summary:
      'Một phiên được sao chép sang máy tính khác sau mỗi lượt: việc chuyển phiên diễn ra tức thì, và phiên có thể tiếp tục ở đó trong khi máy này ngủ.',
    intro:
      'Một phiên mà bạn có thể sẽ tiếp tục ở nơi khác có thể giữ một bản sao ở đó: sau mỗi lượt của phiên, phần mới được sao chép sang máy tính kia. Khi đó việc chuyển phiên sang đó diễn ra tức thì, và nếu máy này đang ngủ hoặc vắng mặt, máy kia có thể tiếp tục từ bản sao. Tính năng này tắt cho đến khi bạn bật nó.',
    keywords: [
      'bản sao',
      'giữ bản sao',
      'các bản sao',
      'tiếp tục từ bản sao',
      'máy đang ngủ',
      'ngoại tuyến',
      'chuyển tức thì',
      'giới hạn dung lượng',
    ],
    sections: [
      {
        heading: 'Bật tính năng',
        steps: [
          'Mở menu **⋮** của phiên và chọn **Giữ bản sao trên…**.',
          'Đánh dấu các máy tính sẽ giữ bản sao; mỗi máy cho biết bản sao của nó đang ở trạng thái nào.',
          'Nhấn **Lưu**.',
        ],
        tips: [
          'Cho mọi phiên của một dự án: **Máy tính › Tùy chọn › Từng dự án**, phần tùy chọn của dự án, **Giữ bản sao các phiên của dự án trên**. Lựa chọn riêng của một phiên trong menu của nó được ưu tiên; **Theo dự án của nó** quay về lựa chọn của dự án.',
        ],
      },
      {
        heading: 'Cần những gì',
        points: [
          'Quyền **Di chuyển** mà máy kia cấp cho máy này: một bản sao là nửa đầu của một lần chuyển phiên.',
          'Cả hai máy đã bật tính năng Máy tính. Khi một trong hai máy tắt tính năng này, máy kia ngoại tuyến hoặc thiếu quyền, việc sao chép tạm dừng và cho biết lý do. Không có gì được xếp hàng chờ: khi trở ngại đó hết, lần sao chép tiếp theo gửi mọi phần đã thêm trong lúc đó.',
        ],
      },
      {
        heading: 'Khi nào, và những gì được gửi',
        points: [
          'Sau một lượt, không bao giờ giữa chừng: khi phiên đã rảnh và yên lặng được năm giây.',
          'Chỉ phần mới: các dòng mới của cuộc trò chuyện, các dòng mới của tác tử con và quy trình của nó, các ảnh chụp lịch sử tệp mới, danh sách việc cần làm. Một tệp hội thoại bị cắt ngắn hoặc bị viết lại sẽ được gửi lại toàn bộ.',
          'Không bao giờ có mã nguồn (mã nguồn đi theo một lần chuyển phiên), thông tin đăng nhập hay cài đặt. Bản sao được giữ trong thư mục dữ liệu của Mother Claude ở máy đó, không bao giờ trong các thư mục của Claude Code.',
        ],
      },
      {
        heading: 'Trạng thái',
        terms: [
          {
            term: 'Đã sao chép lúc 14:32',
            text: 'Bản sao ở đó giống phiên sau lượt cuối của nó.',
          },
          {
            term: 'Sẽ sao chép khi lượt của nó kết thúc',
            text: 'Một lượt đang chạy; bản sao sẽ được gửi khi lượt kết thúc.',
          },
          {
            term: 'Tạm dừng',
            text: 'Kèm lý do: máy kia ngoại tuyến, tính năng Máy tính đang tắt trên một trong hai máy, thiếu quyền **Di chuyển**, Mother Claude ở đó cũ hơn, máy đó đã dọn chỗ cho các bản sao mới hơn, hoặc phiên giờ đang ở trên một máy khác.',
          },
        ],
      },
      {
        heading: 'Chuyển phiên tức thì',
        points: [
          'Một lần chuyển sang máy đang giữ bản sao sẽ cập nhật bản sao trước, rồi chỉ gửi danh sách các tệp của phiên kèm dấu vân tay của chúng: máy kia đối chiếu bản sao với danh sách đó và đặt phiên từ bản sao. **Các lần chuyển gần đây** cho biết phiên đến từ bản sao.',
          'Nếu không dùng được bản sao, lần chuyển sẽ gửi cuộc trò chuyện như bình thường.',
        ],
      },
      {
        heading: 'Tiếp tục ở đây từ bản sao',
        steps: [
          'Trên máy giữ bản sao, khi máy của chính phiên đó đang ngoại tuyến: mở phiên (các phiên của máy đó hiện màu xám), hoặc thẻ **Bản sao** trên **Máy tính**.',
          'Nhấn **Tiếp tục ở đây từ bản sao lúc …** (trên thẻ: **Tiếp tục ở đây**), rồi xác nhận.',
        ],
        points: [
          'Từ giờ phiên ở trên máy này: bản sao được đặt vào thư mục Claude Code của máy này, dưới tài khoản cùng tên và trong thư mục mà các thư mục dùng chung của bạn ánh xạ tới, rồi phiên khởi động ở đây với mô hình, mức nỗ lực và chế độ như trước. Mã nguồn là thư mục dự án hiện có của máy này.',
          'Nếu máy đầu tiên vẫn tiếp tục phiên trong lúc vắng mặt, không có gì bị mất: khi phiên được chuyển sang đó lần sau, các lượt riêng của máy đó được giữ bên cạnh.',
        ],
      },
      {
        heading: 'Thẻ Bản sao',
        points: [
          'Thẻ **Bản sao**, ở cuối trang **Máy tính**, liệt kê các bản sao được giữ trên máy này (với **Tiếp tục ở đây** khi máy của chúng ngoại tuyến, và **Gỡ bỏ**) và các phiên của máy này được giữ trên máy khác (với **Ngừng giữ bản sao trên …**, thao tác này cũng gỡ bản sao ở đó).',
          'Mỗi máy giữ tối đa 2 GB bản sao của mỗi máy khác (**Bản sao phiên của các máy khác**, trong **Máy tính › Tùy chọn › Chung**); bản sao được làm mới lâu nhất sẽ bị gỡ trước, không bao giờ là bản sao của một phiên đang ở trên máy đó.',
          'Điện thoại đã ghép nối với máy của phiên có thể chọn nơi giữ bản sao và gỡ chúng; giới hạn dung lượng được đặt trong ứng dụng máy tính.',
        ],
      },
    ],
  },

  // ──────────────────────────────────────────────────── Khắc phục sự cố
  {
    id: 'machines-sync',
    title: 'Giữ một dự án đồng bộ giữa hai máy tính',
    icon: 'refresh',
    group: 'machines',
    summary:
      'Mã nguồn của một dự án được giữ giống nhau trên hai máy tính, theo cả hai chiều: thay đổi trên máy này được chuyển sang máy kia.',
    intro:
      'Làm cùng một dự án trên hai máy tính? Đồng bộ mã nguồn giữ mã nguồn của nó giống nhau trên cả hai: một chỉnh sửa, một tệp mới, một commit hay một lần chuyển nhánh trên máy này sẽ xuất hiện trên máy kia sau vài giây. Tính năng này tắt cho đến khi bạn bật nó, cho từng dự án một, với một máy tính khác.',
    keywords: [
      'đồng bộ mã nguồn',
      'đồng bộ',
      'giữ đồng bộ',
      'hai máy tính',
      'cùng mã nguồn',
      'xung đột',
      'bản được giữ',
      'phản chiếu',
    ],
    sections: [
      {
        heading: 'Bật tính năng',
        where: 'desktop',
        steps: [
          'Mở **Máy tính › Đồng bộ mã nguồn** và nhấn **Giữ một dự án đồng bộ…**.',
          'Chọn **Dự án**, và chọn máy tính kia trong **Với**.',
          'Xem lại các tệp git bỏ qua sẽ đi kèm (**Thay đổi** mở **Tùy chọn › Từng dự án**), rồi nhấn **Giữ đồng bộ**.',
          'Trên máy tính kia, đề nghị hiện trong **Hộp thư** của máy đó và dưới dạng thông báo: người ở máy đó nhấn **Giữ đồng bộ** (hoặc **Để sau**).',
        ],
        tips: [
          'Tính năng chỉ bật khi cả hai máy tính đều đồng ý. Lần đồng bộ đầu tiên chạy ngay sau đó: máy có thay đổi, hoặc có commit mới hơn, gửi chúng sang máy sạch và đang ở phía sau. Khi cả hai đều có thay đổi riêng, bạn chọn giữ bản nào (bên dưới).',
          'Phần tùy chọn riêng của một dự án cũng có lựa chọn này, **Giữ dự án này đồng bộ với …**: trong **Máy tính › Tùy chọn › Từng dự án**, hoặc ở mục **Khi chuyển máy** trong cài đặt của dự án (**Cài đặt dự án…**).',
        ],
        links: [{ label: 'Mở trang Đồng bộ mã nguồn', path: '/computers/sync' }],
      },
      {
        heading: 'Những gì cần có',
        points: [
          'Một thư mục dùng chung ánh xạ dự án tới thư mục của nó trên máy tính kia (**Máy tính › Tùy chọn › Tài khoản và thư mục**). Thư mục ở đó phải là cùng dự án: cùng `origin`, hoặc có chung một commit. Nếu dự án chưa có ở đó, hãy nhấn **Sao chép sang …** trước.',
          'Quyền **Di chuyển** theo cả hai chiều: đồng bộ mã nguồn ghi vào thư mục dự án của mỗi máy.',
          'Cả hai máy tính đều bật tính năng Máy tính.',
        ],
      },
      {
        heading: 'Những gì được chuyển',
        points: [
          'Các commit của nhánh và các thay đổi đã stage, chưa stage và chưa theo dõi, đúng như hiện có: những gì một lần chuyển phiên mang theo. Không có gì bị gộp và không có gì được đẩy lên đâu cả: mã nguồn đi thẳng giữa hai máy tính.',
          'Các tệp git bỏ qua, như tệp `.env`, `node_modules` và kết quả build, ở nguyên trên mỗi máy, trừ những tệp dự án liệt kê trong **Các tệp git bỏ qua sẽ đi kèm**, và chỉ khi máy kia đã cấp cho máy này quyền **Nguy hiểm**. Bản cũ mà một tệp như vậy thay thế ở máy đó được giữ trong thư mục dữ liệu của Mother Claude trên máy đó (mười bản gần nhất).',
          'Vài giây yên lặng sau một thay đổi, máy đã thay đổi sẽ gửi nó đi; một lần kiểm tra mỗi phút bắt những gì bị sót. **Đồng bộ ngay** chạy một lần ngay lập tức.',
        ],
      },
      {
        heading: 'Thẻ Đồng bộ mã nguồn',
        intro:
          '**Máy tính › Đồng bộ mã nguồn** liệt kê từng dự án được giữ đồng bộ, trong **Các dự án được giữ đồng bộ**, và tình trạng của nó:',
        terms: [
          { term: 'Đã đồng bộ', text: 'Lần đồng bộ gần nhất, và theo chiều nào.' },
          { term: 'Đang đồng bộ…', text: 'Các thay đổi đang được chuyển.' },
          {
            term: 'Đang chờ …',
            text: 'Máy tính kia đã được hỏi; chưa có gì được đồng bộ cho đến khi có người ở đó xác nhận.',
          },
          {
            term: 'Tạm dừng',
            text: 'Kèm lý do: máy tính kia đang ngoại tuyến, tính năng Máy tính đang tắt trên một trong hai máy, thiếu một quyền **Di chuyển**, hoặc một phiên làm việc trong dự án đang được chuyển giữa hai máy. Nó tự tiếp tục khi điều đó kết thúc, từ dự án như lúc đó: không có gì bị xếp hàng chờ.',
          },
          {
            term: 'Thay đổi ở cả hai',
            text: 'Cả hai máy tính đều đã thay đổi nó: bạn chọn (bên dưới).',
          },
          {
            term: 'Tắt',
            text: 'Đã tắt trên máy tính kia, hoặc máy đó đã chọn để sau. **Bật lại** hỏi lại; **Gỡ bỏ** xóa nó khỏi máy này.',
          },
        ],
        points: ['**Tắt** dừng tính năng ngay trên máy này và báo cho máy kia.'],
      },
      {
        heading: 'Khi cả hai máy tính đều đã thay đổi',
        intro:
          'Đồng bộ mã nguồn không bao giờ ghi đè: khi cả hai máy tính đều đã thay đổi dự án kể từ lần đồng bộ trước, nó dừng trên cả hai, và thẻ cho biết mỗi bên đã thay đổi bao nhiêu tệp và có commit hay chuyển nhánh hay không.',
        steps: [
          'Nhấn **Giữ bản của máy Mac này** (hoặc **Giữ bản của máy PC này**) hoặc **Giữ bản của …** cho máy tính kia.',
          'Nhấn lần nữa để xác nhận.',
        ],
        points: [
          'Bên bạn không giữ trước tiên được lưu trong chính kho git của nó, dưới `refs/mother-claude/sync/<id>/kept-<time>`: cây làm việc cùng các tệp chưa theo dõi, những gì đã stage và các commit nó đã tạo. Sau đó thư mục dự án của nó trở về trạng thái đồng bộ gần nhất, và các thay đổi của bên được giữ sẽ được chuyển tới.',
          'Thẻ liệt kê từng trạng thái đã lưu. Để xem một trạng thái, trong thư mục dự án của máy đó: `git show --stat refs/mother-claude/sync/<id>/kept-<time>`.',
          'Cùng một thay đổi được thực hiện trên cả hai máy tính không phải là xung đột. Khi việc đặt một bên sang chỗ khác có thể làm mất thứ gì đó (một tệp git bỏ qua nằm cản đường, một submodule đã thay đổi, một lần merge đang dở), không có gì thay đổi và thẻ cho biết lý do.',
        ],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Điện thoại thấy **Máy tính › Đồng bộ mã nguồn** và tình trạng của từng dự án, chỉ để đọc: bật hay tắt, **Đồng bộ ngay** và chọn một bên được thực hiện trong ứng dụng máy tính.',
        ],
      },
    ],
  },

  {
    id: 'machines-run',
    title: 'Chạy lệnh trên máy tính khác',
    icon: 'terminal',
    group: 'machines',
    summary:
      'Cho một phiên build và chạy kiểm thử trên một máy tính khác của bạn, với mã nguồn của chính nó, mà không phải chuyển phiên.',
    intro:
      'Một phiên Claude Code trên máy này có thể chạy lệnh trên máy khác: một phiên trên máy Mac chạy bộ kiểm thử trên máy PC Windows của bạn, với đúng mã nguồn nó đang có, rồi đọc những gì được in ra. Không có gì bị chuyển đi, và bạn không phải đổi sang máy khác. Tính năng này tắt cho đến khi máy chạy lệnh cho phép, và bạn duyệt từng lần chạy.',
    keywords: [
      'chạy lệnh',
      'máy khác',
      'biên dịch',
      'kiểm thử',
      'thư mục làm việc',
      'luôn cho phép',
      'đầu ra',
      'tệp kết quả',
      'hủy lần chạy',
      'lệnh nguy hiểm',
    ],
    sections: [
      {
        heading: 'Bật tính năng',
        where: 'desktop',
        steps: [
          'Trên máy tính sẽ chạy lệnh, mở **Máy tính**.',
          'Trên thẻ của máy tính có các phiên sẽ nhờ chạy lệnh, cạnh **Trên máy PC này, … được** (hoặc **Trên máy Mac này, … được**), nhấn **Thay đổi**.',
          'Đánh dấu **Chạy**, cho **Cùng mạng**, **Internet** hoặc cả hai, rồi nhấn **Lưu**.',
          'Trên máy tính có các phiên nhờ chạy lệnh, hãy giữ **Cấp công cụ của Mother Claude cho các phiên** ở trạng thái bật, trong **Cài đặt › Công cụ cho Claude Code**: các phiên tới được máy kia qua những công cụ đó.',
        ],
        tips: [
          '**Chạy** cần **Điều khiển** và **Xem**: đánh dấu quyền này sẽ đánh dấu cả hai. Quyền này tắt với mọi máy cho đến khi bạn đánh dấu.',
          'Phần thiết lập kiểm tra **Công cụ cho Claude Code**, và **Bật** sẽ bật chúng. Sau đó bài kiểm tra chạy một lệnh `echo` cố định theo mỗi chiều đã được cấp quyền: **Chạy ở đó** và **Chạy ở đây**.',
        ],
        links: [
          { label: 'Mở trang Máy tính', path: '/computers' },
          { label: 'Xem “Công cụ Claude Code”', path: '/help/claude-code-tools' },
        ],
      },
      {
        heading: 'Nhờ một phiên dùng tính năng này',
        intro:
          'Hãy nói bằng lời của bạn, chẳng hạn “chạy bộ kiểm thử trên DESKTOP-PC”. Claude tìm các máy tính của bạn, viết lệnh cho shell của máy đó (zsh trên máy Mac, PowerShell trên Windows) rồi chạy nó. Claude có năm công cụ cho việc này:',
        terms: [
          {
            term: '`machine_list`',
            text: 'Các máy tính khác của bạn: hệ điều hành và shell của chúng, chúng có đang trực tuyến không, và máy này có được chạy lệnh ở đó không.',
          },
          {
            term: '`machine_run`',
            text: 'Chạy một lệnh trên một máy trong số đó, trong thư mục phiên đang làm việc hoặc một thư mục khác của dự án, và chờ tối đa 10 phút.',
          },
          { term: '`machine_wait`', text: 'Chờ tiếp một lần chạy vẫn đang chạy.' },
          {
            term: '`machine_output`',
            text: 'Đọc toàn bộ những gì một lần chạy đã in ra, từng trang một, hoặc đưa một tệp nó tạo ra vào dự án.',
          },
          {
            term: '`machine_cancel`',
            text: 'Dừng một lần chạy, cùng mọi thứ nó đã khởi chạy.',
          },
        ],
        points: [
          'Phiên phải làm việc trong một kho git: mã nguồn của nó được gửi sang máy kia dưới dạng một bản chụp git.',
          'Ở chế độ quyền mặc định của Claude Code, phiên sẽ hỏi trước khi dùng công cụ, như với mọi công cụ của Mother Claude. Việc đó chỉ cho phép phiên hỏi: mỗi lần chạy vẫn phải chờ bạn duyệt, ở mọi chế độ.',
        ],
      },
      {
        heading: 'Duyệt từng lần chạy',
        intro:
          'Mỗi lần chạy đều chờ bạn duyệt trên máy tính có phiên đang nhờ: trong **Hộp thư**, trên trang của phiên và trên điện thoại của bạn. Thẻ yêu cầu duyệt hiện:',
        terms: [
          { term: 'Máy tính', text: 'Nơi lệnh chạy.' },
          {
            term: 'Thư mục',
            text: 'Bản sao dự án trên máy đó, và thư mục bên trong nó. Lệnh chạy trong một thư mục làm việc ở đó, với mã nguồn của phiên: bản thân bản sao không bị động đến.',
          },
          { term: 'Lệnh', text: 'Dòng lệnh, đúng như nó sẽ chạy.' },
          { term: 'Shell', text: 'zsh trên máy Mac, PowerShell trên Windows.' },
          {
            term: 'Giới hạn thời gian',
            text: '30 phút, trừ khi phiên yêu cầu khác; tối đa 2 giờ.',
          },
          {
            term: 'Tệp cần liệt kê',
            text: 'Các tệp do lần chạy tạo ra mà Claude muốn xem, chẳng hạn `target/release/*.exe`.',
          },
        ],
        points: [
          '**Cho phép một lần** để chạy lệnh. **Từ chối** báo Claude đừng thử lại; **Từ chối kèm góp ý…** còn nói cho Claude nên làm gì thay vào đó.',
          'Không ai trả lời trong 10 phút: lệnh không được chạy.',
        ],
      },
      {
        heading: 'Luôn cho phép một lệnh',
        steps: [
          'Trên thẻ yêu cầu duyệt của một lần chạy, nhấn **Cho phép và ghi nhớ…**.',
          'Chọn **Các lệnh từ phiên này**, **Các lệnh từ …** (dự án của phiên) hoặc **Mọi lệnh trên …**; hoặc **Chỉ khi khớp…** để chỉ cho phép các lệnh bắt đầu bằng vài từ, chẳng hạn `cargo test`.',
        ],
        points: [
          'Từ đó trở đi, một lệnh khớp sẽ chạy trên máy đó mà không hỏi. Chỉ tính nguyên từ: `cargo test` bao gồm `cargo test --lib`, nhưng không bao giờ bao gồm `cargo test-foo`.',
          'Một lệnh có chứa `;`, `&`, `|`, `$`, `(`, `)`, `{`, `}`, `<`, `>`, dấu backtick hoặc dấu xuống dòng thì luôn phải hỏi, dù nó bắt đầu bằng gì: không gì có thể đi kèm sau một lệnh đã được cho phép.',
          'Lệnh nguy hiểm không bao giờ được cho phép theo cách này. Một shell hay một trình thông dịch đứng riêng, như `bash`, `pwsh` hay `python`, không phải là một mẫu: hãy ghi chương trình cùng thứ nó chạy, chẳng hạn `npm run build` hay `python -m pytest`.',
          'Ai trả lời được yêu cầu thì cũng cho phép được các lệnh từ phiên này; một dự án, hoặc mọi lệnh, cần quyền thao tác nguy hiểm: ứng dụng máy tính, hoặc điện thoại được phép thực hiện thao tác nguy hiểm.',
          'Chúng được giữ trong Mother Claude, không phải trong cài đặt của Claude Code, cùng các **Quyền** khác của bạn: **Sửa** để thay đổi một quyền, **Gỡ bỏ** để rút lại, và lệnh đó sẽ lại hỏi.',
        ],
      },
      {
        heading: 'Lệnh nguy hiểm',
        points: [
          'Lệnh động đến cấu hình của Claude Code (`.claude/`, `.mcp.json`), dữ liệu của Mother Claude, `~/.ssh`, tệp khởi động shell hay git hook là lệnh nguy hiểm: chỉ ứng dụng máy tính, hoặc điện thoại được phép thực hiện thao tác nguy hiểm, mới cho phép được, và không bao giờ cho phép luôn.',
          'Lệnh đó còn cần quyền **Nguy hiểm** mà máy chạy lệnh cấp cho máy này. Máy đó cũng tự kiểm tra lệnh trước khi bạn được hỏi: lệnh mà chỉ quy tắc của riêng nó coi là nguy hiểm vẫn là một lần chạy nguy hiểm.',
        ],
      },
      {
        heading: 'Lệnh chạy ở đâu',
        points: [
          'Trong một thư mục làm việc riêng trên máy kia: mỗi dự án và mỗi máy nhờ chạy có một thư mục, nằm trong thư mục dữ liệu của Mother Claude ở máy đó. Đó là một git worktree của bản sao dự án trên máy đó, tức thư mục mà các thư mục dùng chung của bạn trỏ tới.',
          'Trước mỗi lần chạy, thư mục làm việc được đặt về đúng mã nguồn hiện tại của phiên: đã commit hay chưa, kể cả các tệp mới. Các tệp git bỏ qua, như `target/` và `node_modules/`, được giữ lại giữa các lần chạy, nên lần build sau tiếp tục từ chỗ lần trước dừng.',
          'Bản sao dự án của chính bạn trên máy đó không bao giờ bị thay đổi: các tệp, nhánh và thay đổi đã stage vẫn giữ nguyên.',
          'Dự án phải có trên máy đó. Nếu chưa có, lần chạy bị từ chối: hãy sao chép dự án sang đó trước, bằng **Sao chép sang …** trong **Máy tính › Tùy chọn › Tài khoản và thư mục**.',
        ],
      },
      {
        heading: 'Lệnh chạy thế nào',
        points: [
          'Trên máy Mac, lệnh chạy trong zsh dưới dạng login shell: những gì `.zprofile` thêm vào `PATH` đều có, nhưng `.zshrc` không được đọc. Trên Windows, lệnh chạy trong PowerShell 7 nếu đã cài, nếu không thì trong Windows PowerShell, không dùng profile PowerShell của bạn.',
          'Lệnh chạy mà không có token hay khóa API của bạn, và không ai gõ gì vào được: một lệnh đặt câu hỏi sẽ thất bại thay vì chờ.',
          'Mỗi thư mục làm việc chỉ có một lần chạy tại một thời điểm: lần sau chờ đến lượt, và giới hạn thời gian của nó vẫn được tính trong lúc chờ.',
          'Khi hết giới hạn thời gian, hoặc khi bạn hủy, lệnh dừng cùng mọi thứ nó đã khởi chạy. Những gì nó để lại chạy nền cũng dừng khi lệnh kết thúc.',
        ],
      },
      {
        heading: 'Những gì nhận lại',
        points: [
          'Claude nhận mã thoát, thời gian chạy và 64 KiB cuối của những gì đã in ra. Lần chạy dài hơn 10 phút sẽ trả lời là vẫn đang chạy, và Claude chờ tiếp.',
          'Toàn bộ những gì đã in ra được giữ một ngày trên máy đã chạy lệnh, và Claude có thể đọc từng trang.',
          'Các tệp lệnh tạo ra được liệt kê kèm kích thước. Claude có thể đưa một tệp, tối đa 64 MiB, vào dự án trong `.mother-claude/artifacts/`, thư mục mà git bỏ qua, cho đến khi lần chạy tiếp theo đặt lại thư mục làm việc.',
        ],
      },
      {
        heading: 'Theo dõi và dừng các lần chạy',
        points: [
          'Trên trang của phiên, một lần chạy là một tác vụ nền: mở **Tác tử**, rồi **Tác vụ nền**. **Đầu ra** hiện những gì nó đã in ra; **Hủy** để dừng nó.',
          '**Máy tính › Hoạt động** liệt kê các lần chạy gần đây theo cả hai chiều trong **Các lần chạy**: máy tính, dự án, lệnh, tình trạng, mã thoát và thời gian. **Đầu ra** hiện phần cuối những gì đã in ra; **Hủy** dừng một lần chạy, từ bất kỳ máy nào trong hai máy.',
          'Mỗi máy giữ các lần chạy của mình một ngày sau khi chúng kết thúc.',
          'Tắt tính năng Máy tính, thu hồi một máy hoặc rút lại quyền **Chạy** của máy đó sẽ dừng ngay những gì máy đó đang chạy ở đây.',
        ],
        links: [{ label: 'Mở trang Hoạt động', path: '/computers/activity' }],
      },
      {
        heading: 'Từ điện thoại',
        points: [
          'Điện thoại trả lời yêu cầu duyệt một lần chạy như mọi yêu cầu khác, theo dõi các lần chạy trên trang của phiên và trong **Các lần chạy**, và hủy được chúng.',
          'Điện thoại cho phép được **Các lệnh từ phiên này**; một dự án, mọi lệnh và các lần chạy nguy hiểm cần quyền thao tác nguy hiểm của điện thoại, được đặt trong ứng dụng máy tính. Trên điện thoại, **Quyền** cũng hiện chúng.',
        ],
      },
      {
        heading: 'Khi lệnh không chạy',
        terms: [
          {
            term: 'Máy kia chưa cấp quyền Chạy cho máy này',
            text: 'Hãy đánh dấu **Chạy** cho máy này trong **Máy tính** trên máy kia, ở cột của loại mạng bạn đang dùng.',
          },
          {
            term: 'Dự án chưa có trên máy kia',
            text: 'Máy đó chưa có bản sao của dự án, hoặc không có thư mục dùng chung nào trỏ tới nó. Hãy sao chép dự án sang đó (**Sao chép sang …**, trong **Máy tính › Tùy chọn › Tài khoản và thư mục**), rồi thử lại.',
          },
          {
            term: 'Cần cập nhật Mother Claude trên …',
            text: 'Mother Claude trên máy đó cũ hơn và chưa chạy lệnh cho máy tính khác: hãy cập nhật nó.',
          },
          {
            term: 'running scripts is disabled on this system',
            text: 'Windows PowerShell tuân theo chính sách thực thi (execution policy) của bạn, vốn chặn `npm.ps1` và các script khác. Hãy cài PowerShell 7 rồi khởi động lại Mother Claude trên máy đó, dùng `npm.cmd`, hoặc cho phép script của chính bạn trên máy đó: `Set-ExecutionPolicy -Scope CurrentUser RemoteSigned`.',
          },
          {
            term: 'Không tìm thấy một chương trình',
            text: 'Trên máy Mac, hãy đặt dòng thêm chương trình vào `PATH` trong `.zprofile`, không chỉ trong `.zshrc`. Trên Windows, hãy khởi động lại Mother Claude sau khi cài chương trình: lần chạy nhận `PATH` từ lúc Mother Claude khởi động.',
          },
        ],
      },
    ],
  },

  {
    id: 'machines-task',
    title: 'Giao tác vụ cho máy tính khác',
    icon: 'send',
    group: 'machines',
    summary:
      'Cho một phiên giao một tác vụ cho một phiên Claude Code trên máy tính khác của bạn, rồi nhận kết quả về dưới dạng một nhánh.',
    intro:
      'Một phiên Claude Code trên máy này có thể giao cả một tác vụ cho một phiên trên máy khác: phiên trên máy Mac nhờ máy PC Windows của bạn làm cho bộ cài build được trên Windows. Một phiên ở đó làm việc này trong bản sao dự án riêng của nó, với mã nguồn của phiên trên máy Mac, và kết quả quay về máy Mac dưới dạng một nhánh, kèm tin nhắn cuối của phiên làm bản tóm tắt. Bạn duyệt từng tác vụ.',
    keywords: ['giao tác vụ', 'tác vụ', 'nhánh', 'tóm tắt', 'phiên khác', 'máy khác', 'ủy thác'],
    sections: [
      {
        heading: 'Bật tính năng',
        points: [
          'Việc giao tác vụ dùng quyền **Chạy**: một phiên làm việc trên máy kia nghĩa là chạy mã ở đó. Hãy bật quyền này giống như khi chạy lệnh.',
          'Cả hai máy đều cần phiên bản Mother Claude này.',
        ],
        links: [{ label: 'Xem “Chạy lệnh trên máy tính khác”', path: '/help/machines-run' }],
      },
      {
        heading: 'Nhờ một phiên dùng tính năng này',
        intro:
          'Hãy nói bằng lời của bạn, chẳng hạn “nhờ DESKTOP-PC làm cho bộ cài build được trên Windows”. Claude viết tác vụ thành một prompt đầy đủ, vì phiên ở đó không biết gì về cuộc trò chuyện của bạn, rồi giao nó đi:',
        terms: [
          {
            term: '`machine_task`',
            text: 'Giao tác vụ đi và chờ tối đa 10 phút để phiên đó xong lượt của mình. Nó có thể chọn chế độ quyền, mô hình và mức nỗ lực của phiên.',
          },
          {
            term: '`machine_wait`',
            text: 'Chờ tiếp một tác vụ vẫn đang chạy, hoặc lần kết thúc tiếp theo của nó khi nó chạy lại.',
          },
          {
            term: '`machine_cancel`',
            text: 'Dừng phiên của tác vụ. Những gì nó đã làm đến lúc đó vẫn được mang về.',
          },
        ],
      },
      {
        heading: 'Duyệt từng tác vụ',
        intro:
          'Mỗi tác vụ đều chờ bạn duyệt trên máy tính có phiên đang nhờ: trong **Hộp thư**, trên trang của phiên và trên điện thoại của bạn. Thẻ yêu cầu duyệt hiện:',
        terms: [
          { term: 'Máy tính', text: 'Nơi tác vụ được giao đến.' },
          {
            term: 'Thư mục',
            text: 'Bản sao dự án trên máy đó. Tác vụ có một worktree riêng của bản sao này, với mã nguồn của phiên: bản thân bản sao không bị động đến.',
          },
          { term: 'Tác vụ', text: 'Toàn bộ prompt mà phiên ở đó nhận được.' },
          {
            term: 'Chế độ quyền',
            text: 'Cách phiên ở đó hỏi trước khi hành động: **default** trừ khi Claude yêu cầu chế độ khác.',
          },
          {
            term: 'Tài khoản',
            text: 'Tài khoản Claude mà phiên chạy dưới đó: tài khoản cùng tên với tài khoản của phiên này, nếu không có thì là tài khoản Mặc định.',
          },
        ],
        points: [
          '**Cho phép và ghi nhớ…** ghi nhớ câu trả lời của bạn: **Các tác vụ từ phiên này**, **Các tác vụ từ …** (dự án của phiên) hoặc **Mọi tác vụ giao cho …**. Một tác vụ nguy hiểm không bao giờ được ghi nhớ.',
          'Một tác vụ mà phiên chạy không cần phê duyệt (`bypassPermissions` hoặc `dontAsk`) là tác vụ nguy hiểm: chỉ ứng dụng máy tính, hoặc điện thoại được phép thực hiện thao tác nguy hiểm, mới cho phép được, và máy kia phải cấp quyền **Nguy hiểm** cho máy này.',
        ],
      },
      {
        heading: 'Tác vụ làm việc ở đâu',
        points: [
          'Trong một worktree riêng trên máy kia, nằm trong thư mục dữ liệu của Mother Claude ở đó: một git worktree của bản sao dự án trên máy đó, đặt về đúng mã nguồn của phiên, dù đã commit hay chưa.',
          'Bản sao dự án của chính bạn ở đó không bao giờ bị thay đổi.',
          'Phiên ở đó là một phiên Claude Code bình thường do Mother Claude chạy. Cả hai máy đều liệt kê nó, đặt tên theo nơi nó đến và dòng đầu của tác vụ; bạn điều khiển nó và trả lời các yêu cầu duyệt của nó từ bất kỳ máy nào.',
        ],
      },
      {
        heading: 'Những gì nhận lại',
        points: [
          'Khi lượt của phiên kết thúc mà không còn gì chờ, những gì nó đã làm, gồm các commit và các thay đổi chưa commit, quay về thành nhánh `mother-claude/<computer>/<task>` trong dự án này.',
          'Nhánh không bao giờ được checkout hay merge: hãy xem nó bằng `git log` và `git diff`, rồi merge những gì bạn muốn, hoặc xóa nó.',
          'Claude cũng nhận tin nhắn cuối của phiên làm bản tóm tắt của tác vụ. Một tác vụ không thay đổi gì thì không tạo nhánh.',
          'Gửi cho phiên thêm một tin nhắn, từ bất kỳ máy nào, và tác vụ chạy lại: khi lượt đó kết thúc, nhánh tiến lên với những gì nó đã làm thêm. Nhánh không bao giờ bị viết lại những gì nó đã có, và một nhánh bạn đã commit lên hoặc đang checkout thì không bao giờ bị dịch chuyển.',
          'Một tác vụ đã xong mà phiên của nó nhàn rỗi 30 phút không có gì để làm thì phiên đó được dừng trên máy kia, giống như các vai trò trong nhóm; worktree vẫn được giữ. Một tin nhắn gửi cho nó từ bất kỳ máy nào sẽ tiếp tục phiên trong worktree của nó, và tác vụ chạy lại.',
        ],
      },
      {
        heading: 'Theo dõi và gỡ bỏ tác vụ',
        points: [
          '**Máy tính › Hoạt động** liệt kê các tác vụ theo cả hai chiều trong **Tác vụ**: nơi đến, dự án, tác vụ, tình trạng và nhánh. **Mở phiên của tác vụ** mở nó trên máy đang chạy nó; **Tóm tắt** hiện tin nhắn cuối của nó; **Hủy** dừng một tác vụ vẫn đang làm việc.',
          '**Gỡ bỏ** xóa một tác vụ đã kết thúc, cùng worktree và phiên của nó trên máy kia. Nhánh của nó vẫn được giữ.',
          'Mỗi máy giữ một tác vụ 7 ngày sau lần kết thúc gần nhất, rồi gỡ nó cùng worktree.',
          'Trên trang của phiên đã giao tác vụ, tác vụ cũng là một tác vụ nền: mở **Tác tử**, rồi **Tác vụ nền**.',
          'Tắt tính năng Máy tính, thu hồi một máy hoặc rút lại quyền **Chạy** của máy đó sẽ hủy các tác vụ máy đó vẫn đang làm ở đây.',
        ],
        links: [{ label: 'Mở trang Hoạt động', path: '/computers/activity' }],
      },
      {
        heading: 'Khi tác vụ không được giao',
        terms: [
          {
            term: 'Máy kia chưa cấp quyền Chạy cho máy này',
            text: 'Hãy đánh dấu **Chạy** cho máy này trong **Máy tính** trên máy kia.',
          },
          {
            term: 'Cần quyền Nguy hiểm của máy kia',
            text: 'Hãy đánh dấu **Nguy hiểm** cho máy này trên máy kia, hoặc để tác vụ chạy ở chế độ mặc định và trả lời các yêu cầu duyệt của nó.',
          },
          {
            term: 'Tài khoản chưa đăng nhập trên máy kia',
            text: 'Hãy đăng nhập tài khoản đó trên máy kia, trong **Tài khoản** ở đó.',
          },
          {
            term: 'Nhánh không tiến lên',
            text: 'Bạn đã commit lên nhánh, hoặc một worktree đang checkout nó: Mother Claude không bao giờ dịch chuyển nó dưới tay bạn. Tác vụ nêu commit mới hơn, và `git branch` tạo một nhánh từ commit đó.',
          },
        ],
      },
    ],
  },

  {
    id: 'machines-troubleshooting',
    title: 'Khắc phục sự cố các máy tính của bạn',
    icon: 'question',
    group: 'troubleshooting',
    summary:
      'Khi hai máy tính không tìm thấy nhau, từ chối một thao tác, hoặc việc chuyển phiên dừng lại.',
    intro: 'Tìm thông báo hoặc hiện tượng bạn gặp, rồi xem cách khắc phục.',
    keywords: [
      'máy tính',
      'ghép nối',
      'ở gần',
      'máy chủ chuyển tiếp',
      'ngoại tuyến',
      'quyền',
      'chuyển phiên',
      'tin cậy',
      'tài khoản',
      'lỗi',
    ],
    sections: [
      {
        heading: 'Thiết lập',
        terms: [
          {
            term: 'Bật tính năng Máy tính bị làm mờ',
            text: 'Còn mục kiểm tra bắt buộc; dòng phía trên nút cho biết còn bao nhiêu. Hãy sửa chúng ở bước **Máy Mac này** (hoặc **Máy PC này**) và **Mọi máy tính**.',
          },
          {
            term: 'Cho phép qua tường lửa không thay đổi gì',
            text: 'Windows hỏi quản trị viên đồng ý trước. Nếu bị từ chối, tường lửa không bị thay đổi: hãy nhấn lại.',
          },
          {
            term: 'Chỉ các phiên của nó phải chờ: phần còn lại vẫn hoạt động.',
            text: 'Tài khoản đó chưa đăng nhập trên máy đó. Các phiên của nó phải chờ ở đó; mọi thứ khác vẫn hoạt động.',
          },
        ],
      },
      {
        heading: 'Ghép nối',
        terms: [
          {
            term: 'Máy kia không có trong Ở gần',
            text: 'Ở gần chỉ cho biết máy nào đang hiện mã: bạn luôn phải nhập mã, và mã dùng được cả khi khác mạng. Trên máy Mac, hãy kiểm tra quyền truy cập Mạng cục bộ của Mother Claude.',
          },
          {
            term: 'Không tìm thấy máy tính nào đang hiện mã này.',
            text: 'Hãy kiểm tra lại mã, và xem cả hai máy có Internet hoặc ở cùng một mạng không. Mỗi mã chỉ dùng được trong năm phút.',
          },
          {
            term: 'Mã đó không khớp.',
            text: 'Hãy kiểm tra lại trên máy tính kia, hoặc tạo mã mới ở đó.',
          },
          {
            term: 'Mã này đã bị hai máy tính dùng, nên không máy nào ghép nối được.',
            text: 'Hãy tạo mã mới.',
          },
          {
            term: 'Đó không phải dãy số trên …',
            text: 'Chưa có gì được ghép nối. Hãy bắt đầu lại, và nhập sáu chữ số mà máy kia hiển thị.',
          },
          {
            term: 'Lượt ghép nối không được xác nhận kịp.',
            text: 'Hãy xác nhận trên cả hai máy trong vòng mười phút kể từ khi tạo mã. Hãy bắt đầu lại.',
          },
        ],
      },
      {
        heading: 'Kết nối',
        terms: [
          {
            term: 'Qua máy chủ chuyển tiếp (chậm hơn), khi ở nhà',
            text: 'Trên máy Mac: quyền truy cập Mạng cục bộ. Trên Windows: loại mạng và tường lửa. Thẻ **Máy Mac này** (hoặc **Máy PC này**) trên **Máy tính** cho biết mục nào.',
          },
          {
            term: '… dùng cài đặt máy chủ chuyển tiếp khác',
            text: 'Hai máy tính dùng máy chủ chuyển tiếp khác nhau có thể không bao giờ gặp được nhau. Nhấn **Dùng cả ở đây** trong **Hộp thư** của một máy, hoặc **Dùng cài đặt này ở mọi nơi** trên máy kia.',
          },
          {
            term: 'Một máy đang ngoại tuyến',
            text: 'Máy đó đang ngủ, đã tắt, hoặc Mother Claude không chạy ở đó. Các phiên của nó vẫn được liệt kê, làm mờ, cho đến khi máy trở lại.',
          },
          {
            term: 'Cũ hơn bản trên máy tính này',
            text: 'Mother Claude trên máy đó cũ hơn: các phiên của nó vẫn hiện, nhưng không gì được chuyển đến hay đi khỏi máy đó cho đến khi bạn cập nhật.',
          },
          {
            term: 'Tính năng Máy tính đang tắt trên …',
            text: 'Hãy bật nó trong **Máy tính** trên máy đó (hoặc trên máy này, khi thông báo nói đến máy này). Khi tính năng này tắt, các máy kia chỉ ghép nối và chạy thiết lập được với máy đó.',
          },
        ],
      },
      {
        heading: 'Quyền',
        terms: [
          {
            term: '… chỉ cho máy tính này xem các phiên của nó, không cho điều khiển.',
            text: 'Quyền **Điều khiển** mà máy đó cấp cho máy này đang tắt: hãy đổi trong **Máy tính** trên máy đó.',
          },
          {
            term: '… không cho máy tính này thực hiện thao tác nguy hiểm trên đó.',
            text: 'Quyền **Nguy hiểm** mà máy đó cấp cho máy này đang tắt: hãy đổi trên máy đó. Điện thoại còn cần quyền thao tác nguy hiểm của chính nó trên máy mà nó ghép nối.',
          },
          {
            term: '… không cho máy tính này chuyển các phiên của nó.',
            text: 'Quyền **Di chuyển** mà máy đó cấp cho máy này đang tắt với loại mạng bạn đang dùng: hãy đổi trên máy đó.',
          },
          {
            term: '… đã từ chối máy tính này',
            text: 'Hãy kiểm tra trên máy đó xem hai máy còn được ghép nối không. Nếu máy đó đã thu hồi máy này, hãy ghép nối lại.',
          },
        ],
      },
      {
        heading: 'Chuyển phiên',
        terms: [
          {
            term: '… không có tài khoản nào tên …',
            text: 'Hãy thêm tài khoản cùng tên trên máy đó (**Tài khoản**) và đăng nhập. Phiên của tài khoản Mặc định sẽ tiếp tục với tài khoản Mặc định của máy đó.',
          },
          {
            term: '… chưa đăng nhập trên …',
            text: 'Hãy đăng nhập trên máy đó (trang **Tài khoản** ở đó), rồi thử lại.',
          },
          {
            term: '… chưa tin cậy …',
            text: 'Hãy mở thư mục đó một lần bằng Claude Code trên máy đó, hoặc mang theo sự tin cậy của thư mục (cần quyền thao tác nguy hiểm).',
          },
          {
            term: '… có thay đổi riêng',
            text: 'Hãy dùng thư mục riêng (worktree), hoặc commit hay stash các thay đổi ở đó trước.',
          },
          { term: '… đang chạy phiên này.', text: 'Hãy dừng nó trên máy đó trước.' },
          {
            term: 'Nó chạy không hỏi quyền',
            text: 'Phiên ở chế độ bỏ qua quyền cần quyền thao tác nguy hiểm ở đây, và quyền **Nguy hiểm** mà máy kia cấp cho máy này.',
          },
          {
            term: 'Remote … không phản hồi … mà không yêu cầu đăng nhập.',
            text: 'Hãy đăng nhập remote trên máy đó (Git Credential Manager, hoặc `gh auth login`) rồi thử lại, hoặc chuyển mã nguồn trực tiếp giữa các máy tính.',
          },
          {
            term: 'Mang theo các tệp git bỏ qua cần …',
            text: 'Cần quyền thao tác nguy hiểm ở nơi phiên đang ở và quyền **Nguy hiểm** của máy nhận. Hoặc bỏ đánh dấu **Mang theo chúng**: các tệp ở lại chỗ cũ.',
          },
          {
            term: 'Không chuyển được …',
            text: 'Không mất gì: phiên vẫn ở nguyên chỗ, và thông báo cho biết lý do. **Thử lại** đưa bạn về các lựa chọn.',
          },
        ],
      },
    ],
  },
];

/** Vietnamese Help topics (docs/I18N.md "Help"), by topic id. */
export const VI_TOPICS_C: Partial<Record<string, HelpTopicSource>> = Object.fromEntries(
  TOPICS.map((t) => [t.id, t]),
);
