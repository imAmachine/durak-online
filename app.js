const entry = document.body.dataset.admin === 'page' ? 'admin-main' : 'main';
import(`/js/${entry}.js?entry=${entry}&v=${Date.now()}`);
